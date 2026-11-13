"""Flush behaviour for handlers attached to :class:`FemtoLogger`.

Covers both halves of ``logger.flush_handlers()``: records queued by the logger
must reach its handlers before the flush is reported, and the flush result must
follow Python's convention that a handler reports failure by raising rather
than by returning a value.
"""

from __future__ import annotations

import time
import typing as typ

import pytest

from femtologging import FemtoLogger, FileHandlerBuilder
from tests.logger_support import (
    FLUSH_TEST_HOOKS_AVAILABLE,
    CollectingHandler,
    HandlerFlushGate,
    RecordCollectingHandler,
    handler_payloads,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

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
