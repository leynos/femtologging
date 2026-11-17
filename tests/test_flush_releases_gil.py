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
from femtologging import _femtologging_rs as rust
from tests.flush_gil_scripts import (
    BUFFERED_RECORDS_SCRIPT,
    FLUSH_FORMATTER_SCRIPT,
    FLUSH_HANDLERS_SCRIPT,
)

# These hooks exist only when the extension was built with the Rust `test-util`
# feature, which the test builds enable.
GIL_RELEASE_OBSERVATION_AVAILABLE = all(
    callable(getattr(rust, name, None))
    for name in (
        "note_python_thread_ran_for_test",
        "gil_release_observations_for_test",
        "reset_gil_release_observations_for_test",
        "wait_for_wait_in_flight_for_test",
    )
)

requires_gil_release_observation = pytest.mark.skipif(
    not GIL_RELEASE_OBSERVATION_AVAILABLE,
    reason="requires Rust extension built with the 'test-util' feature",
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


def _run_flush_formatter_script(tmp_path: Path, mode: str) -> None:
    """Run the blocked-formatter script in a fresh interpreter."""
    result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed script
        [
            sys.executable,
            "-c",
            FLUSH_FORMATTER_SCRIPT,
            str(tmp_path / "app.log"),
            mode,
        ],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stderr


@requires_gil_release_observation
def test_flush_handlers_releases_the_gil_for_a_python_formatter(
    tmp_path: Path,
) -> None:
    """``flush_handlers()`` lets other Python threads run while it waits.

    The handler is registered from Python, so ``flush_handlers()`` reaches it
    through ``PyHandler::flush()``. Its worker is parked in a Python formatter
    that only another Python thread can release, and that thread waits until a
    wait has the interpreter detached. The extension marks that span in flight
    and records an observation only while it lasts, so the release cannot land
    in front of the native wait -- not even in the detached section
    ``flush_handlers()`` keeps around the whole call -- and a flush that kept
    the GIL would block the release, time the formatter out, and record no
    observations at all. The count is kept on the Rust side and advanced only
    during that span, so neither time before the wait nor the handover after
    the call returns can be counted. The file is read before ``close()``, so
    the record can be there only because the flush waited for the handler. A
    fresh interpreter keeps the logger's dispatch shard idle, so the record is
    admitted rather than refused behind earlier tests' work.

    This call also drains the dispatch shard, and that drain waits detached
    too, so it can release the GIL before the native wait starts. The isolated
    case below measures the native wait on its own.
    """
    _run_flush_formatter_script(tmp_path, "dispatched")


@requires_gil_release_observation
def test_flush_handlers_native_wait_releases_the_gil(
    tmp_path: Path,
) -> None:
    """The native wait releases the GIL with no dispatch drain in front of it.

    ``flush_handlers()`` drains the logger's dispatch shard first, and that
    drain waits detached too, so it could release the GIL and hide a native
    wait that held it. Queuing the record on the handler directly leaves the
    logger's shard with nothing in flight, so the drain returns at once and the
    only wait left is the one that reaches the handler: if that wait held the
    GIL, the release could not land and no observation would be recorded.
    """
    _run_flush_formatter_script(tmp_path, "direct")


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
