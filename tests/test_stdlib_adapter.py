"""Construction and record-dispatch tests for :class:`StdlibHandlerAdapter`.

Covers validation of the wrapped handler, translation of femtologging
records into stdlib ``LogRecord`` output, level mapping, producer
``contextvars`` propagation, and forwarding of exception and stack payloads.
The wrapped handler's own level threshold is covered in
``tests/test_stdlib_adapter_levels.py``; attribute population, delegation,
and export checks live in ``tests/test_stdlib_adapter_records.py``.
"""

from __future__ import annotations

import contextvars
import io
import logging
import threading
import time
import typing as typ

import pytest

from femtologging import FemtoLogger, StdlibHandlerAdapter
from femtologging.adapter import TRACE_LEVEL_NUM
from tests.stdlib_adapter_support import (
    AdapterProbe,
    HandlerSetup,
    LevelCase,
    LogRequest,
    build_adapter_probe,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

_REQUEST_ID = contextvars.ContextVar[str | None]("request_id", default=None)


def _poll_until(predicate: cabc.Callable[[], bool], timeout: float) -> bool:
    """Poll *predicate* with a monotonic deadline and make one final check."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(0.01)
    return predicate()


def _wait_for(predicate: cabc.Callable[[], bool], timeout: float = 1.0) -> bool:
    """Return whether *predicate* succeeds before the deadline."""
    return bool(_poll_until(predicate, timeout))


def _assert_eventually(
    predicate: cabc.Callable[[], bool],
    message: str,
    timeout: float = 1.0,
) -> None:
    """Assert that *predicate* succeeds before the monotonic deadline."""
    assert _wait_for(predicate, timeout), message


@pytest.fixture
def probe() -> AdapterProbe:
    """Return an adapter wrapping a StreamHandler over a fresh StringIO."""
    return build_adapter_probe()


class TestStdlibHandlerAdapterConstruction:
    """Verify adapter construction validates the wrapped handler."""

    @staticmethod
    def test_wraps_stdlib_handler(probe: AdapterProbe) -> None:
        """Adapter should forward records to the wrapped stdlib handler."""
        probe.handler.setFormatter(logging.Formatter("%(message)s"))
        probe.adapter.handle_record(
            {"logger": "test", "level": "INFO", "message": "wrapped-ok"},
        )
        probe.handler.flush()

        output = probe.stream.getvalue()
        assert "wrapped-ok" in output, (
            f"wrapped handler did not receive the record: {output!r}"
        )

    @staticmethod
    def test_trace_level_registered_on_init() -> None:
        """TRACE level should be registered with stdlib logging after adapter init."""
        StdlibHandlerAdapter(logging.StreamHandler(io.StringIO()))
        assert logging.getLevelName(TRACE_LEVEL_NUM) == "TRACE", (
            f"TRACE level not registered: "
            f"getLevelName({TRACE_LEVEL_NUM}) = "
            f"{logging.getLevelName(TRACE_LEVEL_NUM)!r}"
        )

    @staticmethod
    @pytest.mark.parametrize(
        "not_a_handler",
        ["not a handler", object()],
        ids=["string", "plain_object"],
    )
    def test_rejects_non_handler(not_a_handler: object) -> None:
        """Adapter should raise TypeError for objects that are not handlers."""
        with pytest.raises(TypeError, match=r"expected a logging\.Handler"):
            StdlibHandlerAdapter(typ.cast("logging.Handler", not_a_handler))


class TestHandleRecordDispatch:
    """Verify that femtologging records are translated and emitted."""

    @staticmethod
    def test_basic_message_emitted(probe: AdapterProbe) -> None:
        """A simple log message should appear in the wrapped handler's output."""
        output = probe.capture(
            LogRequest("myapp", "INFO", "hello world"),
            HandlerSetup(
                formatter=logging.Formatter("%(name)s %(levelname)s %(message)s")
            ),
        )
        assert output == "myapp INFO hello world\n", (
            f"unexpected formatted output: {output!r}"
        )

    @staticmethod
    @pytest.mark.parametrize(
        "case",
        [
            LevelCase("ERROR", "ERROR", "failure"),
            LevelCase("DEBUG", "DEBUG", "trace detail", "DEBUG", logging.DEBUG),
            LevelCase("CRITICAL", "CRITICAL", "fatal"),
            LevelCase("WARN", "WARNING", "caution"),
            LevelCase("TRACE", "TRACE", "lowest", "TRACE", TRACE_LEVEL_NUM),
        ],
        ids=["error", "debug", "critical", "warn", "trace"],
    )
    def test_level_mapped(probe: AdapterProbe, case: LevelCase) -> None:
        """Each femtologging level should map to its stdlib equivalent."""
        output = probe.capture(
            LogRequest("app", case.level, case.message, case.logger_level),
            HandlerSetup(
                formatter=logging.Formatter("%(levelname)s %(message)s"),
                handler_level=case.handler_level,
            ),
        )
        assert output == f"{case.expected_level} {case.message}\n", (
            f"level {case.level!r} did not map to {case.expected_level!r}: {output!r}"
        )


class TestContextVarPropagation:
    """Verify stdlib handlers retain producer ``contextvars`` values."""

    @staticmethod
    def test_filter_observes_producer_contextvar(probe: AdapterProbe) -> None:
        """A worker-dispatched filter should observe the caller's context."""

        class RequestIdFilter(logging.Filter):
            @typ.override
            def filter(self, record: logging.LogRecord) -> bool:
                """Attach the current request ID to the stdlib record."""
                record.correlation_id = _REQUEST_ID.get() or "-"
                return True

        probe.handler.addFilter(RequestIdFilter())
        probe.handler.setFormatter(
            logging.Formatter("cid=%(correlation_id)s %(message)s")
        )
        logger = FemtoLogger("contextvars.single")
        logger.add_handler(probe.adapter)

        token = _REQUEST_ID.set("REQ-42")
        try:
            logger.info("via adapter")
        finally:
            _REQUEST_ID.reset(token)

        _assert_eventually(
            lambda: "cid=REQ-42 via adapter" in probe.stream.getvalue(),
            f"filter did not observe producer context: {probe.stream.getvalue()!r}",
        )
        assert logger.flush_handlers(), "logger worker did not flush"

    @staticmethod
    def test_each_thread_keeps_its_contextvar_value() -> None:
        """Queued records should retain their respective producer contexts."""
        observed: list[tuple[str, str | None]] = []
        observed_lock = threading.Lock()

        class ObservingHandler(logging.Handler):
            """Invoke attached filters without retaining emitted records."""

            @typ.override
            def emit(self, record: logging.LogRecord) -> None:
                """Discard a record after its filters have observed the context."""

        class ContextFilter(logging.Filter):
            @typ.override
            def filter(self, record: logging.LogRecord) -> bool:
                """Record the message and request ID seen by this filter."""
                with observed_lock:
                    observed.append((record.getMessage(), _REQUEST_ID.get()))
                return True

        handler = ObservingHandler()
        handler.addFilter(ContextFilter())
        logger = FemtoLogger("contextvars.multi")
        logger.add_handler(StdlibHandlerAdapter(handler))
        expected = {f"message-{index}": f"request-{index}" for index in range(4)}

        def emit(message: str, request_id: str) -> None:
            """Emit one message with its request ID set in this thread."""
            token = _REQUEST_ID.set(request_id)
            try:
                logger.info(message)
            finally:
                _REQUEST_ID.reset(token)

        threads = [
            threading.Thread(target=emit, args=(message, request_id))
            for message, request_id in expected.items()
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()

        _assert_eventually(
            lambda: len(observed) == len(expected),
            f"expected {len(expected)} filter invocations, got {observed!r}",
        )
        assert logger.flush_handlers(), "logger worker did not flush"
        assert dict(observed) == expected, (
            f"context values crossed threads: {observed!r}"
        )


class TestExceptionForwarding:
    """Verify that exception information reaches the stdlib handler."""

    @staticmethod
    def _raise_value_error(msg: str = "test error") -> None:
        """Raise a ValueError for testing exc_info capture."""
        raise ValueError(msg)

    @staticmethod
    def test_exc_info_forwarded_as_text(probe: AdapterProbe) -> None:
        """Exception payload should appear as exc_text on the LogRecord."""
        output = ""
        try:
            TestExceptionForwarding._raise_value_error()
        except ValueError:
            output = probe.capture(
                LogRequest("app", "ERROR", "caught", exc_info=True),
                HandlerSetup(formatter=logging.Formatter("%(message)s\n%(exc_text)s")),
            )
        assert "caught" in output, f"expected 'caught' in output: {output!r}"
        assert "ValueError" in output, f"expected 'ValueError' in output: {output!r}"

    @staticmethod
    def test_stack_info_forwarded(probe: AdapterProbe) -> None:
        """Stack trace payload should appear as stack_info on the LogRecord."""
        output = probe.capture(
            LogRequest("app", "INFO", "trace", stack_info=True),
            HandlerSetup(formatter=logging.Formatter("%(message)s")),
        )
        assert "trace" in output, f"expected 'trace' in output: {output!r}"
        # stdlib Formatter appends stack_info after the message
        assert "Stack (most recent call last)" in output, (
            f"expected stack trace header in output: {output!r}"
        )
