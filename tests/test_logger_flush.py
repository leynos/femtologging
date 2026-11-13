"""Flush behaviour for handlers attached to :class:`FemtoLogger`.

Covers both halves of ``logger.flush_handlers()``: records queued by the logger
must reach its handlers before the flush is reported, and the flush result must
follow Python's convention that a handler reports failure by raising rather
than by returning a value.
"""

from __future__ import annotations

import collections.abc as cabc
import time
import typing as typ

import pytest

from femtologging import (
    FemtoFileHandler,
    FemtoLogger,
    FemtoRotatingFileHandler,
    FemtoSocketHandler,
    FemtoStreamHandler,
    FemtoTimedRotatingFileHandler,
    FileHandlerBuilder,
    RotatingFileHandlerBuilder,
    SocketHandlerBuilder,
    StreamHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
)
from tests.logger_support import (
    FLUSH_TEST_HOOKS_AVAILABLE,
    CollectingHandler,
    HandlerFlushGate,
    RecordCollectingHandler,
    handler_payloads,
)

if typ.TYPE_CHECKING:
    from pathlib import Path

type _NativeHandlerFactory = cabc.Callable[
    [Path],
    FemtoStreamHandler
    | FemtoFileHandler
    | FemtoRotatingFileHandler
    | FemtoTimedRotatingFileHandler
    | FemtoSocketHandler,
]

# Long enough to outlast the logger's detached drain of its own queue, and well
# inside the native handler's one-second flush deadline.
_FORMATTER_GATE_SECONDS = 0.4


@pytest.mark.parametrize(
    "handler_type",
    [RecordCollectingHandler, CollectingHandler],
    ids=["handle_record", "legacy_handle"],
)
def test_flush_handlers_releases_gil_for_python_handlers(
    handler_type: type[RecordCollectingHandler] | type[CollectingHandler],
) -> None:
    """Flush immediately delivers records to either Python handler protocol."""
    if not FLUSH_TEST_HOOKS_AVAILABLE:
        pytest.skip("flush synchronization regression requires the test-util feature")

    logger_name = "python.handlers"
    logger = FemtoLogger(logger_name)
    handler = handler_type()
    messages = ["first", "second", "third"]
    expected_payloads = [(logger_name, "INFO", message) for message in messages]

    try:
        with HandlerFlushGate(handler, logger):
            logger.add_handler(handler)
            for message in messages:
                logger.info(message)

            # Enqueue, then flush before inspecting; polling first masks the defect.
            assert logger.flush_handlers(), "logger worker did not flush"
            payloads = handler_payloads(handler)
            assert payloads == expected_payloads, f"bad payloads: {payloads!r}"
    finally:
        logger.remove_handler(handler)


def test_native_handler_flush_releases_gil_for_python_formatter(
    tmp_path: Path,
) -> None:
    """A native handler's flush must not starve its own Python formatter.

    ``FemtoFileHandler`` formats records on its worker thread, and a Python
    formatter has to reacquire the GIL to run. A logger that waits for the
    worker while holding the GIL therefore cannot receive the flush
    acknowledgement before its one-second deadline: the worker is stuck
    waiting for the interpreter the waiter is holding.
    """
    path = tmp_path / "native_flush.log"

    def slow_formatter(record: cabc.Mapping[str, object]) -> str:
        """Render a record slowly enough to expose a flush that holds the GIL.

        Sleeping releases the GIL, so a flush that waits detached lets this
        formatter finish while one that waits attached starves it.

        Returns
        -------
        str
            The formatted record, matching the handler's default layout.
        """
        time.sleep(_FORMATTER_GATE_SECONDS)
        return f"{record['logger']} [{record['level']}] {record['message']}"

    handler = FileHandlerBuilder(str(path)).with_formatter(slow_formatter).build()
    logger = FemtoLogger("native.flush")
    try:
        logger.add_handler(handler)
        logger.info("formatted during the flush")
        assert logger.flush_handlers(), (
            "the flush must wait for the formatter without holding the GIL"
        )
        assert path.read_text(encoding="utf-8") == (
            "native.flush [INFO] formatted during the flush\n"
        ), "the flush reported success before the record reached the file"
    finally:
        logger.remove_handler(handler)
        handler.close()


def test_logger_flush_handlers_flushes_attached_file_handler(
    tmp_path: Path,
) -> None:
    """A file handler attached through Python is flushed by the logger.

    ``FemtoFileHandler`` exposes no Python ``flush`` method, so the wrapper
    must forward to the native handler. Otherwise the logger would report a
    successful flush while the record stayed in the handler's buffer.
    """
    path = tmp_path / "logger_flush.log"
    logger = FemtoLogger("core")
    handler = FemtoFileHandler(str(path), flush_interval=10_000)
    try:
        logger.add_handler(handler)
        logger.info("buffered")
        assert logger.flush_handlers(), "logger flush must report success"
        assert path.read_text(encoding="utf-8") == "core [INFO] buffered\n", (
            "the flush reported success before the record reached the file"
        )
    finally:
        logger.remove_handler(handler)
        handler.close()


def test_logger_flush_handlers_flushes_attached_stream_handler(
    capfd: pytest.CaptureFixture[str],
) -> None:
    """A stream handler attached through Python is flushed by the logger.

    The stream worker formats and writes on its own thread, so reporting a
    successful flush before the worker has answered would leave the record
    invisible to the caller.
    """
    handler = StreamHandlerBuilder.stderr().build()
    logger = FemtoLogger("stream.flush")
    try:
        logger.add_handler(handler)
        logger.info("streamed")
        assert logger.flush_handlers(), "logger flush must report success"
        assert "stream.flush [INFO] streamed" in capfd.readouterr().err, (
            "the flush reported success before the record reached the stream"
        )
    finally:
        logger.remove_handler(handler)
        handler.close()


@pytest.mark.parametrize(
    "builder_type",
    [RotatingFileHandlerBuilder, TimedRotatingFileHandlerBuilder],
    ids=["rotating", "timed-rotating"],
)
def test_logger_flush_handlers_flushes_attached_rotation_handler(
    builder_type: type[RotatingFileHandlerBuilder]
    | type[TimedRotatingFileHandlerBuilder],
    tmp_path: Path,
) -> None:
    """A rotation handler attached through Python is flushed by the logger.

    Both rotation handlers buffer records until their flush interval, so the
    logger must reach the native flush rather than assume the wrapper drained
    itself.
    """
    path = tmp_path / "rotation_flush.log"
    handler = builder_type(str(path)).with_flush_after_records(10_000).build()
    logger = FemtoLogger("rotation.flush")
    try:
        logger.add_handler(handler)
        logger.info("rotated")
        assert logger.flush_handlers(), "logger flush must report success"
        assert path.read_text(encoding="utf-8") == (
            "rotation.flush [INFO] rotated\n"
        ), "the flush reported success before the record reached the file"
    finally:
        logger.remove_handler(handler)
        handler.close()


@pytest.mark.parametrize(
    "build_handler",
    [
        pytest.param(lambda _path: StreamHandlerBuilder.stderr().build(), id="stream"),
        pytest.param(
            lambda path: FemtoFileHandler(str(path), flush_interval=10_000),
            id="file",
        ),
        pytest.param(
            lambda path: RotatingFileHandlerBuilder(str(path)).build(),
            id="rotating",
        ),
        pytest.param(
            lambda path: TimedRotatingFileHandlerBuilder(str(path)).build(),
            id="timed-rotating",
        ),
        pytest.param(
            lambda path: SocketHandlerBuilder().with_unix_path(str(path)).build(),
            id="socket",
        ),
    ],
)
def test_logger_flush_handlers_reports_a_closed_native_handler(
    build_handler: _NativeHandlerFactory,
    tmp_path: Path,
) -> None:
    """A native flush that cannot reach its worker reports failure.

    Each kind's native ``flush`` returns ``False`` once the handler is closed.
    The logger must pass that through instead of treating every native flush
    as a success, which would silently discard the records it claimed to
    drain. This is the case that tells a constant "success" apart from a flush
    that actually consulted the handler.
    """
    handler = build_handler(tmp_path / "closed.sink")
    handler.close()
    logger = FemtoLogger("closed.flush")
    try:
        logger.add_handler(handler)
        assert not logger.flush_handlers(), (
            "a closed native handler must make the logger flush report failure"
        )
    finally:
        logger.remove_handler(handler)


class _RecordingFlushHandler(CollectingHandler):
    """Base handler that records whether ``flush()`` was called."""

    def __init__(self) -> None:
        """Initialize the record buffer and the flush counter."""
        super().__init__()
        self.flush_calls = 0


class _FlushReturningNoneHandler(_RecordingFlushHandler):
    """Handler whose ``flush()`` returns ``None``, as Python handlers do."""

    def flush(self) -> None:
        """Succeed silently, matching ``logging.Handler.flush``."""
        self.flush_calls += 1


class _FlushRaisingHandler(_RecordingFlushHandler):
    """Handler whose ``flush()`` reports failure by raising."""

    def flush(self) -> None:
        """Fail the way a broken Python handler would."""
        self.flush_calls += 1
        message = "flush exploded"
        raise RuntimeError(message)


class _FlushReturningFalseHandler(_RecordingFlushHandler):
    """Handler that reports failure through a falsy return value."""

    def flush(self) -> bool:
        """Refuse the flush without raising."""
        self.flush_calls += 1
        return False


def test_python_handler_flush_returning_none_reports_success() -> None:
    """A ``None`` return means the handler flushed without complaint.

    ``logging.Handler.flush`` and ``io.IOBase.flush`` both return ``None`` on
    success, so treating a falsy return as failure would report a broken flush
    for every well-behaved Python handler.
    """
    logger = FemtoLogger("flush.none")
    handler = _FlushReturningNoneHandler()

    try:
        logger.add_handler(handler)
        assert logger.flush_handlers(), "a None-returning flush must succeed"
    finally:
        logger.remove_handler(handler)

    assert handler.flush_calls == 1, "the handler's flush was not invoked"


@pytest.mark.parametrize(
    "handler_type",
    [
        pytest.param(_FlushRaisingHandler, id="raises"),
        pytest.param(_FlushReturningFalseHandler, id="returns-false"),
    ],
)
def test_python_handler_flush_failure_reports_failure(
    handler_type: type[_RecordingFlushHandler],
) -> None:
    """Either failure style makes the logger's flush report failure.

    A handler may raise, as Python's flush protocol expects, or return a
    falsy value of its own accord. Both are failures the logger must report.
    """
    logger = FemtoLogger(f"flush.{handler_type.__name__}")
    handler = handler_type()

    try:
        logger.add_handler(handler)
        assert not logger.flush_handlers(), "a failing flush must report failure"
    finally:
        logger.remove_handler(handler)

    assert handler.flush_calls == 1, "the handler's flush was not invoked"


class _SensitiveFlushRaisingHandler(_RecordingFlushHandler):
    """Handler whose flush exception quotes content that must not be logged."""

    def __init__(self, marker: str) -> None:
        """Store the marker the exception message will carry."""
        super().__init__()
        self.marker = marker

    def flush(self) -> None:
        """Raise with a message that stands in for a record payload."""
        self.flush_calls += 1
        raise RuntimeError(self.marker)


def test_python_flush_exception_text_does_not_reach_stderr(
    capsys: pytest.CaptureFixture[str],
) -> None:
    """A handler's exception message must not be printed or logged.

    The message comes from the handler and may quote a record, so the flush
    path reports only a bounded category. Printing the exception, or logging
    its text, would leak this marker to the operator.
    """
    logger = FemtoLogger("flush.sensitive")
    marker = "SENSITIVE-flush-payload-7d1c"
    handler = _SensitiveFlushRaisingHandler(marker)

    try:
        logger.add_handler(handler)
        assert not logger.flush_handlers(), "a raising flush must report failure"
    finally:
        logger.remove_handler(handler)

    assert handler.flush_calls == 1, "the handler's flush was not invoked"
    stderr = capsys.readouterr().err
    assert marker not in stderr, f"the flush path leaked the exception: {stderr!r}"
