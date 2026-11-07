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

        def handle(self, logger: str, level: str, message: str) -> None: ...
        def flush(self) -> bool: ...
        def close(self) -> None: ...


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
