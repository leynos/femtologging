"""The scenario programs that exercise the file-handler interpreter lock.

Issue #441: a direct ``handler.handle()`` call under a blocking overflow policy
waits for queue capacity while holding the GIL, so a slow or stalled file
worker starves every other Python thread.

This module owns the shared harness and the renderer that produce each child
interpreter's program. ``file_handler_gil_wait_scenarios.py`` supplies the
bodies for the waits themselves, and ``file_handler_gil_support.py`` owns the
parent-side runner, explaining how the progress-counting verdict those programs
reach is arrived at.
"""

from __future__ import annotations

import textwrap
import typing as typ

from .file_handler_gil_close_scenarios import (
    CLOSE_DURING_BODIES,
    CONCURRENT_CLOSE_BODY,
)
from .file_handler_gil_wait_scenarios import (
    BLOCK_HOLD_SECONDS,
    CLOSE_HOLD_SECONDS,
    TIMEOUT_MS,
    WAIT_BODIES,
)

if typ.TYPE_CHECKING:
    from pathlib import Path

#: Budget for one wait inside the child. No scenario needs more than two
#: seconds of waiting, so this only elapses once something has already gone
#: wrong. Staying under the parent's
#: :data:`~tests.file_handler_gil_support.SCENARIO_TIMEOUT_SECONDS` lets the
#: child report the real cause before the parent gives up on it.
CHILD_WAIT_SECONDS = 10.0

#: Iterations the watchdog spins through between progress records; a few
#: milliseconds of bytecode per record.
PROGRESS_ITERATIONS = 200_000

#: Records the watchdog must land inside a blocked window before the GIL counts
#: as released. A released wait of a second or more yields hundreds; a wait
#: that holds the GIL yields none, so the bar is set low enough to be immune to
#: scheduling noise and still far above what the defect can produce.
MIN_PROGRESS_RECORDS = 5

#: How long to let a started call reach its wait before the watchdog begins.
#: The call cannot return while its queue is full or its worker is parked, so a
#: live call at this point is inside the binding, and no progress record can be
#: attributed to the moment before it entered.
SETTLE_SECONDS = 0.05

#: Shortest window that counts as the wait a scenario pins. Every wait under
#: test lasts about a second, so a call that returns sooner did not perform the
#: wait at all, and counting progress inside its window would prove nothing.
MIN_WAIT_SECONDS = 0.5

#: How long the formatter parks the worker before giving up and proceeding.
#: This bounds every scenario: it outlasts the widest wait under test (the
#: two-second overflow timeout), so no scenario depends on it, while a binding
#: that holds the GIL still reaches its assertion instead of hanging.
WORKER_PARK_SECONDS = 6.0


#: Harness shared by every scenario. A blocking formatter parks the file
#: worker so that the queue stays full and a blocking policy really waits.
_HARNESS = """
import sys
import threading
import time

from femtologging import (
    FileHandlerBuilder,
    OverflowPolicy,
    RotatingFileHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
)

# A capacity of 1 lets exactly one record queue behind the one being formatted,
# which is the state a blocking policy has to wait on.
WORKER_STARTED = threading.Event()
RELEASE_WORKER = threading.Event()
STOP_WATCHDOG = threading.Event()

PROGRESS_ITERATIONS = {iterations}
MIN_PROGRESS_RECORDS = {records}
CHILD_WAIT = {child_wait}
WORKER_PARK = {worker_park}
MIN_WAIT = {min_wait}
SETTLE = {settle}

PROGRESSED_AT = []
ENTERED_AT = []
RETURNED_AT = []


def blocking_formatter(record):
    if not WORKER_STARTED.is_set():
        WORKER_STARTED.set()
        # Park the worker so the queue stays full. The timeout is a safety
        # valve, not an expectation: every scenario releases the worker well
        # within it. It also means a binding that holds the GIL through its
        # wait still limps to the assertion below instead of hanging until the
        # parent gives up, so the failure names the GIL rather than a timeout.
        RELEASE_WORKER.wait(timeout=WORKER_PARK)
    return f"{{record['logger']}} [{{record['level']}}] {{record['message']}}"


def run_watchdog():
    \"\"\"Record a timestamp every few milliseconds until told to stop.\"\"\"
    # A pure-bytecode spin is enough: the watchdog only has to prove it was
    # scheduled at all inside the blocked window.
    while not STOP_WATCHDOG.is_set():
        for _ in range(PROGRESS_ITERATIONS):
            pass
        PROGRESSED_AT.append(time.monotonic())


# Callers must not wait for progress after this call. Under the defect the
# calling thread holds the GIL for its whole wait, so a wait here would itself
# be starved until the blocked call had already finished -- and the scenario
# would report a deadlock that had already come and gone.
def start_watchdog():
    \"\"\"Start the progress-recording thread.\"\"\"
    watchdog = threading.Thread(target=run_watchdog, name="watchdog", daemon=True)
    watchdog.start()
    return watchdog


def start_watchdog_when_blocked(blocked):
    \"\"\"Start the watchdog only once ``blocked`` is inside its wait.

    The call under test cannot return while its queue is full or its worker is
    parked, so a call still alive after the settle period is parked in the
    binding. Starting the watchdog afterwards means no record it makes can have
    been taken before the call entered, which a window recorded on the calling
    thread alone cannot rule out.
    \"\"\"
    time.sleep(SETTLE)
    assert blocked.is_alive(), (
        "the call was not running when the watchdog was due to start: either it "
        "returned early or it held the GIL and starved this thread until its "
        "wait ended"
    )
    return start_watchdog()


def start_blocked(call):
    \"\"\"Run ``call`` on its own thread, recording the call's own window.\"\"\"

    def wrapper():
        ENTERED_AT.append(time.monotonic())
        try:
            call()
        finally:
            RETURNED_AT.append(time.monotonic())

    thread = threading.Thread(target=wrapper, name="blocked")
    thread.start()
    return thread


def join_blocked(thread):
    \"\"\"Wait for a blocked call to return, failing if it never does.\"\"\"
    thread.join(timeout=CHILD_WAIT)
    assert not thread.is_alive(), "the blocked handler call never returned"


def stop_watchdog(watchdog):
    STOP_WATCHDOG.set()
    watchdog.join(timeout=CHILD_WAIT)
    assert not watchdog.is_alive(), "the watchdog never stopped"


def assert_progress_during(call_name, watchdog):
    \"\"\"Fail unless other threads ran freely while ``call_name`` was blocked.\"\"\"
    stop_watchdog(watchdog)
    entered = ENTERED_AT[0]
    returned = RETURNED_AT[0]
    waited = returned - entered
    # A call that never waited would trivially collect progress records from
    # its own brief window, so pin the window before counting them.
    assert waited >= MIN_WAIT, (
        f"{{call_name}} returned after only {{waited:.3f}}s, too short to be the "
        f"wait this scenario pins (at least {{MIN_WAIT}}s), so the GIL-release "
        "verdict would be vacuous"
    )
    during = [stamp for stamp in PROGRESSED_AT if entered <= stamp <= returned]
    assert len(during) >= MIN_PROGRESS_RECORDS, (
        f"a second Python thread recorded only {{len(during)}} progress "
        f"record(s) during the {{returned - entered:.3f}}s wait in {{call_name}} "
        f"(expected at least {{MIN_PROGRESS_RECORDS}}), so the binding held the "
        "GIL and starved other threads until its wait expired"
    )


def build_handler(path, policy, capacity=1):
    return (
        {builder}
        .with_capacity(capacity)
        .with_flush_after_records(10_000)
        .with_overflow_policy(policy)
        .with_formatter(blocking_formatter)
        .build()
    )


def saturate(handler):
    \"\"\"Park the worker on one record, then queue a second behind it.

    At the default capacity of 1 the queue is now full; a capacity of 2 leaves
    one free command slot for the flush scenarios to use.
    \"\"\"
    handler.handle("core", "INFO", "first")
    assert WORKER_STARTED.wait(CHILD_WAIT), "the worker never reached the formatter"
    handler.handle("core", "INFO", "second")


def guard(body):
    \"\"\"Run ``body``, releasing the worker however it finishes.\"\"\"
    try:
        body()
    finally:
        # Always release the worker: a parked formatter would otherwise leave a
        # handler that cannot shut down inside the scenario's own teardown.
        RELEASE_WORKER.set()
"""


#: Every builder produces a handler whose ``handle()`` funnels into the same
#: core enqueue, so all of them must share one GIL contract. The rotating
#: thresholds are pushed far out of reach: rotation must not be what unblocks
#: the wait under test.
_BUILDERS = {
    "file": "FileHandlerBuilder({path!r})",
    "rotating": (
        "RotatingFileHandlerBuilder({path!r})"
        ".with_max_bytes(1 << 40)"
        ".with_backup_count(1)"
    ),
    "timed-rotating": (
        "TimedRotatingFileHandlerBuilder({path!r})"
        ".with_when('H')"
        ".with_interval(1 << 20)"
    ),
}

#: The exception each handler raises when a ``"timeout:N"`` wait expires. The
#: file and rotating bindings use ``RuntimeError``; the timed-rotating binding
#: maps the same underlying error to ``ValueError``. Naming the difference here
#: keeps the scenario honest about the contract it is pinning, and documents the
#: inconsistency rather than hiding it behind a broad catch.
_ERROR_TYPES = {
    "file": "RuntimeError",
    "rotating": "RuntimeError",
    "timed-rotating": "ValueError",
}

#: The scenario bodies, keyed by the label their failures report. The waits
#: themselves live in the sibling module; the close bodies cover the borrow
#: states a close can produce.
_BODIES = {
    **WAIT_BODIES,
    "concurrent-close": CONCURRENT_CLOSE_BODY,
    **CLOSE_DURING_BODIES,
}

#: Builder labels the test module parametrizes over.
BUILDER_NAMES: tuple[str, ...] = tuple(sorted(_BUILDERS))


def render_scenario(builder_name: str, body_name: str, path: Path) -> str:
    """Render a scenario as a standalone child-interpreter program.

    Parameters
    ----------
    builder_name : str
        Registered builder label, such as ``"file"`` or ``"timed-rotating"``.
    body_name : str
        Registered scenario label, such as ``"handle-timeout"``.
    path : pathlib.Path
        Output path the child scenario passes to its handler builder.

    Returns
    -------
    str
        Complete Python source for the selected scenario and shared harness.

    Examples
    --------
    Render the flush scenario and check that the harness and the selected body
    both reach the program:

    >>> from pathlib import Path
    >>> script = render_scenario("file", "flush", Path("flush.log"))
    >>> "def blocking_formatter" in script
    True
    >>> 'assert_progress_during("flush", watchdog)' in script
    True
    """
    script = _HARNESS.format(
        iterations=PROGRESS_ITERATIONS,
        records=MIN_PROGRESS_RECORDS,
        child_wait=CHILD_WAIT_SECONDS,
        worker_park=WORKER_PARK_SECONDS,
        min_wait=MIN_WAIT_SECONDS,
        settle=SETTLE_SECONDS,
        builder=_BUILDERS[builder_name].format(path=str(path)),
    )
    return script + textwrap.dedent(
        _BODIES[body_name].format(
            path=str(path),
            timeout_ms=TIMEOUT_MS,
            hold=BLOCK_HOLD_SECONDS,
            close_hold=CLOSE_HOLD_SECONDS,
            error_type=_ERROR_TYPES[builder_name],
        )
    )
