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

# The handler's worker formats the record in Python and waits for a release
# that only another Python thread can give, so a flush that holds the GIL
# deadlocks until the formatter times out. The release waits for the flush to
# begin, so the test cannot pass before there is a flush to exercise.
FLUSH_FORMATTER_SCRIPT = """
import pathlib
import sys
import threading

from femtologging import FileHandlerBuilder, get_logger

path = pathlib.Path(sys.argv[1])
formatter_entered = threading.Event()
flush_started = threading.Event()
release_formatter = threading.Event()


def pause_formatter(record):
    formatter_entered.set()
    if not release_formatter.wait(timeout=5):
        raise TimeoutError("no other thread could release the formatter")
    return f"formatted {record['message']}"


def release():
    formatter_entered.wait(timeout=5)
    # Wait for the flush itself. Releasing as soon as the formatter is entered
    # would free it during dispatch, before there is a flush to test.
    flush_started.wait(timeout=5)
    release_formatter.set()


handler = FileHandlerBuilder(str(path)).with_formatter(pause_formatter).build()
logger = get_logger("gil.flush.formatter")
logger.add_handler(handler)
threading.Thread(target=release, daemon=True).start()

logger.info("paused")
# The main thread holds the GIL from here into flush_handlers(), so a worker
# released after this point still cannot finish the formatter unless the flush
# lets go of the GIL.
flush_started.set()
assert not release_formatter.is_set(), (
    "the formatter was released before flush_handlers() began"
)
assert logger.flush_handlers(), "flush_handlers stalled behind the GIL"
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
    """``flush_handlers()`` lets another thread release a blocked formatter.

    The handler is registered from Python, so ``flush_handlers()`` reaches it
    through ``PyHandler::flush()``. Its worker is parked in a Python formatter
    that only another Python thread can release; that thread can run only if
    the flush releases the GIL, and the flush can finish only if the formatter
    does. The file is read before ``close()``, so the record can be there only
    because the flush waited for the handler: a ``PyHandler::flush()`` that did
    not delegate would return with the formatter still blocked, and the read
    would find an empty file. A fresh interpreter keeps the logger's dispatch
    shard idle, so the record is admitted rather than refused behind earlier
    tests' work.
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
