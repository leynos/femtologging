"""The scenario programs that exercise the file-handler interpreter lock.

Issue #441: a direct ``handler.handle()`` call under a blocking overflow policy
waits for queue capacity while holding the GIL, so a slow or stalled file
worker starves every other Python thread.

This module owns the program each child interpreter runs;
``file_handler_gil_support.py`` owns the parent-side runner, and explains how
the progress-counting verdict those programs reach is arrived at.
"""

from __future__ import annotations

import textwrap
import typing as typ

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

#: How long the ``"block"`` scenario waits before releasing the parked worker.
#: The call cannot return before then, so this is a guaranteed-blocked window.
BLOCK_HOLD_SECONDS = 1.0

#: How long the formatter parks the worker before giving up and proceeding.
#: This bounds every scenario: it outlasts the widest wait under test (the
#: two-second overflow timeout), so no scenario depends on it, while a binding
#: that holds the GIL still reaches its assertion instead of hanging.
WORKER_PARK_SECONDS = 6.0

#: The overflow timeout the scenarios configure. It mirrors the binding's own
#: unit, milliseconds, so the scenario and the wait it exercises agree.
_TIMEOUT_MS = 2000


#: Harness shared by every scenario. A blocking formatter parks the file
#: worker so that the queue stays full and a blocking policy really waits.
_HARNESS = """
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
    during = [stamp for stamp in PROGRESSED_AT if entered <= stamp <= returned]
    assert len(during) >= MIN_PROGRESS_RECORDS, (
        f"a second Python thread recorded only {{len(during)}} progress "
        f"record(s) during the {{returned - entered:.3f}}s wait in {{call_name}} "
        f"(expected at least {{MIN_PROGRESS_RECORDS}}), so the binding held the "
        "GIL and starved other threads until its wait expired"
    )


def build_handler(path, policy):
    return (
        {builder}
        .with_capacity(1)
        .with_flush_after_records(10_000)
        .with_overflow_policy(policy)
        .with_formatter(blocking_formatter)
        .build()
    )


def saturate(handler):
    \"\"\"Park the worker on one record, then fill the single queue slot.\"\"\"
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


#: An unbounded ``"block"`` wait ends only when the worker is released, so the
#: window is guaranteed to span :data:`BLOCK_HOLD_SECONDS`.
_BLOCK_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    saturate(handler)
    watchdog = start_watchdog()
    blocked = start_blocked(lambda: handler.handle("core", "INFO", "third"))
    try:
        # The call cannot return while the worker is parked and the queue is
        # full, so this window is a genuine blocked wait.
        time.sleep({hold})
    finally:
        RELEASE_WORKER.set()
    join_blocked(blocked)

    assert_progress_during("handle", watchdog)
    handler.close()


guard(body)
"""


#: The timeout policy ends its own wait, so progress must have been recorded
#: while the call was still outstanding.
_TIMEOUT_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.timeout({timeout_ms}))
    failures = []

    def call():
        try:
            handler.handle("core", "INFO", "third")
        except {error_type} as err:
            failures.append(err)

    saturate(handler)
    watchdog = start_watchdog()
    blocked = start_blocked(call)
    join_blocked(blocked)

    assert_progress_during("handle", watchdog)
    assert len(failures) == 1, (
        "a timed-out handle() must raise exactly once, got "
        f"{{len(failures)}} failure(s)"
    )
    assert "timed out" in str(failures[0]), (
        "a saturated timeout policy must report the timeout, got "
        f"{{failures[0]!r}}"
    )
    RELEASE_WORKER.set()
    handler.close()


guard(body)
"""


#: ``flush()`` waits for the worker's acknowledgement, so a parked worker parks
#: the caller until the one-second deadline expires.
_FLUSH_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    results = []
    saturate(handler)
    watchdog = start_watchdog()
    blocked = start_blocked(lambda: results.append(handler.flush()))
    join_blocked(blocked)

    # ``flush`` gives up on the worker after one second, so the window this
    # asserts over is the binding's own deadline.
    assert_progress_during("flush", watchdog)
    assert results == [False], (
        "a flush that outlives its one-second deadline must report False, got "
        f"{{results!r}}"
    )
    RELEASE_WORKER.set()
    handler.close()


guard(body)
"""


#: ``close()`` waits for the worker to stop, and abandons a worker that does not
#: acknowledge within its deadline.
_CLOSE_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    saturate(handler)
    watchdog = start_watchdog()
    blocked = start_blocked(handler.close)
    join_blocked(blocked)

    assert_progress_during("close", watchdog)
    RELEASE_WORKER.set()
    handler.close()


guard(body)
"""


#: A blocked ``handle()`` keeps its shared borrow of the handler for the whole
#: detached wait, so a concurrent ``close()`` cannot take the exclusive borrow
#: it needs. Only the exception type is matched, to stay independent of PyO3's
#: message wording. This scenario waits for the blocked call to finish before
#: closing, so it never depends on the outcome of the race.
_CONCURRENT_CLOSE_BODY = """
def body():
    handler = build_handler({path!r}, OverflowPolicy.block())
    saturate(handler)
    watchdog = start_watchdog()
    blocked = start_blocked(lambda: handler.handle("core", "INFO", "third"))
    try:
        # ``handle`` is stuck on queue capacity, so it cannot have returned and
        # the borrow is still held when ``close`` asks for it.
        time.sleep({handoff})

        try:
            handler.close()
        except RuntimeError:
            pass
        else:
            raise AssertionError(
                "close() took the handler while handle() was blocked in it"
            )
    finally:
        RELEASE_WORKER.set()

    join_blocked(blocked)
    assert_progress_during("handle", watchdog)
    # With the blocked call finished the borrow is free, so shutdown succeeds.
    handler.close()


guard(body)
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

#: The scenario bodies, keyed by the label their failures report.
_BODIES = {
    "handle-block": _BLOCK_BODY,
    "handle-timeout": _TIMEOUT_BODY,
    "flush": _FLUSH_BODY,
    "close": _CLOSE_BODY,
    "concurrent-close": _CONCURRENT_CLOSE_BODY,
}

#: Builder labels the test module parametrizes over.
BUILDER_NAMES: tuple[str, ...] = tuple(sorted(_BUILDERS))


def render_scenario(builder_name: str, body_name: str, path: Path) -> str:
    """Render one scenario into a standalone program for a child interpreter."""
    script = _HARNESS.format(
        iterations=PROGRESS_ITERATIONS,
        records=MIN_PROGRESS_RECORDS,
        child_wait=CHILD_WAIT_SECONDS,
        worker_park=WORKER_PARK_SECONDS,
        builder=_BUILDERS[builder_name].format(path=str(path)),
    )
    return script + textwrap.dedent(
        _BODIES[body_name].format(
            path=str(path),
            timeout_ms=_TIMEOUT_MS,
            hold=BLOCK_HOLD_SECONDS,
            handoff=BLOCK_HOLD_SECONDS,
            error_type=_ERROR_TYPES[builder_name],
        )
    )
