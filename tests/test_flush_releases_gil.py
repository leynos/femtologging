"""Regression tests: blocking flush and close calls must release the GIL.

A worker thread needs the GIL to call a Python formatter, and the logger's
worker needs it to dispatch to any handler attached from Python. A ``flush()``,
``close()``, or ``flush_handlers()`` call that waited while holding the GIL
stalled until its timeout, returned ``False``, and, for ``close()``, abandoned
the records still queued.
"""

from __future__ import annotations

import subprocess  # ruff: ignore[suspicious-subprocess-import] runs this interpreter
import sys
import typing as typ

import pytest

from femtologging import (
    FileHandlerBuilder,
    RotatingFileHandlerBuilder,
    StreamHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

    class _QueuedHandler(typ.Protocol):
        """The handler surface these tests drive."""

        def handle(self, logger: str, level: str, message: str) -> None:
            """Queue one record for the handler's worker."""

        def flush(self) -> bool:
            """Wait for the worker to flush queued records."""

        def close(self) -> None:
            """Stop the worker and wait for it to finish."""


def _python_formatter(record: cabc.Mapping[str, object]) -> str:
    """Render a record through Python so the worker must take the GIL."""
    return f"formatted {record['message']}"


# Checks the drop counter first, so a refused record cannot pass for a flush
# that returned too early.
FLUSH_HANDLERS_SCRIPT = """
import pathlib
import sys

from femtologging import FemtoFileHandler, get_logger

path = pathlib.Path(sys.argv[1])
handler = FemtoFileHandler(str(path))
logger = get_logger("gil.flush")
logger.add_handler(handler)

logger.info("delivered")
assert logger.flush_handlers(), "flush_handlers stalled behind the GIL"
assert logger.get_dropped() == 0, "the record was refused before dispatch"
assert path.read_text(encoding="utf-8") == "gil.flush [INFO] delivered\\n", (
    "flush_handlers() returned before the record reached the file"
)
handler.close()
"""

# The worker is told to flush only every third record, so the one record logged
# here stays in its buffer. That makes delivery proof of delegation rather than
# of timing: no thread, clock, or scheduler can write the file, so a
# `PyHandler::flush()` that reported success without asking the handler to
# flush would leave the file empty. The empty read before the call is the
# precondition that proves the record really was still buffered.
BUFFERED_RECORDS_SCRIPT = """
import pathlib
import sys

from femtologging import FileHandlerBuilder, get_logger

path = pathlib.Path(sys.argv[1])
handler = FileHandlerBuilder(str(path)).with_flush_after_records(3).build()
logger = get_logger("gil.flush.buffered")
logger.add_handler(handler)

logger.info("held")
assert path.read_text(encoding="utf-8") == "", (
    "the record reached the file before flush_handlers(), so this test cannot "
    "show that the flush is what delivered it"
)
assert logger.flush_handlers(), "flush_handlers did not report the flush"
assert logger.get_dropped() == 0, "the record was refused before dispatch"
assert path.read_text(encoding="utf-8") == "gil.flush.buffered [INFO] held\\n", (
    "flush_handlers() returned without flushing the buffered record"
)
handler.close()
"""

# The handler's worker formats the record in Python and waits for a release
# only another Python thread can give. The release thread is started once the
# formatter is already blocked, and gives way after a delay far longer than
# the flush needs to reach the handler, so the release can only happen while
# the flush is waiting on that handler. A flush that held the GIL through the
# wait would block the release and the ticker thread below, and the formatter
# would time out. The ticker counts the interpreter time the flush lets other
# Python code have, which fails a GIL-holding flush even if the formatter
# somehow escaped.
FLUSH_FORMATTER_SCRIPT = """
import pathlib
import sys
import threading
import time

from femtologging import FileHandlerBuilder, get_logger

path = pathlib.Path(sys.argv[1])
formatter_entered = threading.Event()
release_formatter = threading.Event()
ticker_started = threading.Event()
is_finished = threading.Event()
ticks = [0]

TICK_SECONDS = 0.005
RELEASE_DELAY_SECONDS = 0.5
FORMATTER_TIMEOUT_SECONDS = 5.0


def pause_formatter(record):
    formatter_entered.set()
    if not release_formatter.wait(timeout=FORMATTER_TIMEOUT_SECONDS):
        raise TimeoutError("no other thread could release the formatter")
    return f"formatted {record['message']}"


def release_after_delay():
    time.sleep(RELEASE_DELAY_SECONDS)
    release_formatter.set()


def count_ticks():
    ticker_started.set()
    while not is_finished.is_set():
        ticks[0] += 1
        time.sleep(TICK_SECONDS)


handler = FileHandlerBuilder(str(path)).with_formatter(pause_formatter).build()
logger = get_logger("gil.flush.formatter")
logger.add_handler(handler)

ticker = threading.Thread(target=count_ticks, daemon=True)
ticker.start()
assert ticker_started.wait(timeout=5), "the ticker thread never started"

logger.info("paused")
assert formatter_entered.wait(timeout=5), "the handler's worker never formatted"

# Start the release only now, so its delay is measured from a point at which
# the flush is the next thing to run. Set before the delay, a release could
# fire before the flush reached the handler and would prove nothing.
release = threading.Thread(target=release_after_delay, daemon=True)
release.start()

ticks_before = ticks[0]
started = time.monotonic()
assert logger.flush_handlers(), "flush_handlers stalled behind the GIL"
elapsed = time.monotonic() - started
is_finished.set()
ticker.join()

assert elapsed >= 0.2, (
    f"flush_handlers() returned after {elapsed:.3f}s, so the blocked formatter "
    "was not what the flush waited for and this test proved nothing"
)
observed = ticks[0] - ticks_before
assert observed >= 10, (
    f"another Python thread ran only {observed} times during {elapsed:.3f}s of "
    "flush_handlers(), so the wait held the GIL"
)
# Read before close(): the record must be on disk because the flush waited for
# the handler, not because close() later drained the worker. If PyHandler's
# flush returned without delegating, the formatter would still be blocked here
# and this read would find an empty file.
assert path.read_text(encoding="utf-8") == "formatted paused\\n", (
    "flush_handlers() returned before the record reached the file"
)
handler.close()
"""

FILE_BUILDERS = {
    "file": FileHandlerBuilder,
    "rotating": RotatingFileHandlerBuilder,
    "timed-rotating": TimedRotatingFileHandlerBuilder,
}


def _build_file_handler(kind: str, path: Path) -> _QueuedHandler:
    """Build a file-backed handler of ``kind`` with a Python formatter."""
    builder = FILE_BUILDERS[kind](str(path)).with_formatter(_python_formatter)
    return typ.cast("_QueuedHandler", builder.build())


@pytest.mark.parametrize("kind", sorted(FILE_BUILDERS))
def test_close_writes_records_formatted_by_python(kind: str, tmp_path: Path) -> None:
    """``close()`` lets the worker format and write every queued record."""
    path = tmp_path / "app.log"
    handler = _build_file_handler(kind, path)
    handler.handle("app", "INFO", "first")
    handler.handle("app", "INFO", "second")
    handler.close()
    assert path.read_text(encoding="utf-8") == "formatted first\nformatted second\n", (
        f"{kind} close() dropped queued records"
    )


@pytest.mark.parametrize("kind", sorted(FILE_BUILDERS))
def test_flush_succeeds_with_a_python_formatter(kind: str, tmp_path: Path) -> None:
    """``flush()`` returns ``True`` while the worker runs a Python formatter."""
    handler = _build_file_handler(kind, tmp_path / "app.log")
    try:
        handler.handle("app", "INFO", "queued")
        assert handler.flush(), f"{kind} flush stalled behind the GIL"
    finally:
        handler.close()


def test_stream_flush_succeeds_with_a_python_formatter() -> None:
    """A stream handler's ``flush()`` does not stall behind the GIL."""
    handler = StreamHandlerBuilder.stderr().with_formatter(_python_formatter).build()
    try:
        handler.handle("app", "INFO", "queued")
        assert handler.flush(), "stream flush stalled behind the GIL"
    finally:
        handler.close()


def test_flush_handlers_releases_the_gil_for_a_python_formatter(
    tmp_path: Path,
) -> None:
    """``flush_handlers()`` lets other Python threads run while it waits.

    The handler is registered from Python, so ``flush_handlers()`` reaches it
    through ``PyHandler::flush()``. Its worker is parked in a Python formatter
    that only another Python thread can release. The release is requested once
    the formatter is already blocked and takes effect half a second later, so
    it can only land while the flush waits on that handler: a flush that kept
    the GIL would block the release, time the formatter out, and leave the
    ticker thread silent. The ticker counts how often another Python thread
    ran during the call, and the file is read before ``close()``, so the
    record can be there only because the flush waited for the handler. A fresh
    interpreter keeps the logger's dispatch shard idle, so the record is
    admitted rather than refused behind earlier tests' work.
    """
    result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed script
        [sys.executable, "-c", FLUSH_FORMATTER_SCRIPT, str(tmp_path / "app.log")],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stderr


def test_flush_handlers_succeeds_straight_after_logging(tmp_path: Path) -> None:
    """``flush_handlers()`` delivers a record logged immediately before it.

    Dispatch shards are shared across a process, so a record can be refused
    when earlier tests have left its shard busy. A fresh interpreter gives this
    scenario a pool of its own.
    """
    result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed script
        [sys.executable, "-c", FLUSH_HANDLERS_SCRIPT, str(tmp_path / "app.log")],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stderr


def test_flush_handlers_delivers_a_buffered_native_record(tmp_path: Path) -> None:
    """``flush_handlers()`` flushes a record the worker was still buffering.

    The handler is registered from Python, so the call reaches it through
    ``PyHandler::flush()``. Its flush threshold is three records and only one
    was logged, so nothing but that flush can put the record on disk. This
    pins the delegation itself, without relying on a blocked formatter or a
    delay: a wrapper that reported success without asking the handler to flush
    would leave the file empty.
    """
    result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed script
        [sys.executable, "-c", BUFFERED_RECORDS_SCRIPT, str(tmp_path / "app.log")],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stderr
