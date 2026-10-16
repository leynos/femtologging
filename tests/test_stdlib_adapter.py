"""Construction and record-dispatch tests for :class:`StdlibHandlerAdapter`.

Covers validation of the wrapped handler, translation of femtologging
records into stdlib ``LogRecord`` output, level mapping, and forwarding of
exception and stack payloads. Attribute population, delegation, and export
checks live in ``tests/test_stdlib_adapter_records.py``.
"""

from __future__ import annotations

import contextvars
import dataclasses
import io
import logging
import threading
import time
import typing as typ

import pytest

from femtologging import FemtoLogger, StdlibHandlerAdapter
from femtologging.adapter import TRACE_LEVEL_NUM

if typ.TYPE_CHECKING:
    import collections.abc as cabc

_REQUEST_ID = contextvars.ContextVar[str | None]("request_id", default=None)


@dataclasses.dataclass(frozen=True, slots=True)
class _LevelCase:
    """A single level-mapping test case."""

    level: str
    expected_level: str
    message: str
    logger_level: str | None = None
    handler_level: int | None = None


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


@dataclasses.dataclass(frozen=True, slots=True)
class _LogRequest:
    """One log emission routed through the adapter under test."""

    logger_name: str
    level: str
    message: str
    logger_level: str | None = None
    stack_info: bool = False
    exc_info: bool = False


@dataclasses.dataclass(frozen=True, slots=True)
class _HandlerSetup:
    """Optional stdlib handler configuration applied before logging."""

    formatter: logging.Formatter | None = None
    handler_level: int | None = None


@dataclasses.dataclass(frozen=True, slots=True)
class _HandlerLevelCase:
    """One record level routed through a wrapped handler at a fixed level."""

    level: str
    message: str
    expected_levelname: str | None


class _CountingHandler(logging.Handler):
    """Handler that counts ``handle()`` calls and captures emitted records.

    ``Handler.handle()`` is overridden rather than ``emit()`` so the test can
    observe how many times the adapter entered the handler's filter-and-lock
    path, which is the observable the level gate is allowed to change.
    """

    def __init__(self) -> None:
        """Initialise with zero handle calls and no captured records."""
        super().__init__()
        self.handle_calls = 0
        self.records: list[logging.LogRecord] = []

    @typ.override
    def handle(self, record: logging.LogRecord) -> bool:
        self.handle_calls += 1
        return super().handle(record)

    @typ.override
    def emit(self, record: logging.LogRecord) -> None:
        self.records.append(record)


class _CountingFilter(logging.Filter):
    """Filter that counts how often the handler consults it."""

    def __init__(self) -> None:
        """Initialise with a zero filter-call count."""
        super().__init__()
        self.calls = 0

    @typ.override
    def filter(self, record: logging.LogRecord) -> bool:
        self.calls += 1
        return True


class _RejectingFilter(logging.Filter):
    """Filter that rejects every record and counts the consultations."""

    def __init__(self) -> None:
        """Initialise with a zero filter-call count."""
        super().__init__()
        self.calls = 0

    @typ.override
    def filter(self, record: logging.LogRecord) -> bool:
        self.calls += 1
        return False


def _dispatch(adapter: StdlibHandlerAdapter, level: str, message: str) -> None:
    """Send one record straight to *adapter* without a worker thread.

    Calling ``handle_record()`` directly keeps the counting assertions
    deterministic; going through a ``FemtoLogger`` would defer dispatch to a
    worker thread and make the counts racy.

    Parameters
    ----------
    adapter
        The adapter under test.
    level
        Femtologging level name for the record.
    message
        Message text for the record.

    Examples
    --------
    >>> handler = _CountingHandler()
    >>> handler.setLevel(logging.ERROR)
    >>> _dispatch(StdlibHandlerAdapter(handler), "ERROR", "boom")
    >>> [record.getMessage() for record in handler.records]
    ['boom']

    """
    adapter.handle_record({"logger": "app", "level": level, "message": message})


@dataclasses.dataclass(frozen=True, slots=True)
class AdapterProbe:
    """A StringIO sink, its stdlib handler, and the adapter wrapping it."""

    stream: io.StringIO
    handler: logging.StreamHandler[io.StringIO]
    adapter: StdlibHandlerAdapter

    def capture(self, request: _LogRequest, setup: _HandlerSetup | None = None) -> str:
        """Log one message via the adapter and return the captured output.

        Parameters
        ----------
        request
            The record to emit and the logger threshold to apply.
        setup
            Optional formatter and handler-level configuration.

        Returns
        -------
        str
            Everything the wrapped handler wrote to the stream.

        Examples
        --------
        >>> stream = io.StringIO()
        >>> handler = logging.StreamHandler(stream)
        >>> probe = AdapterProbe(stream, handler, StdlibHandlerAdapter(handler))
        >>> probe.capture(_LogRequest("doctest.adapter", "INFO", "hi")).strip()
        'hi'

        """
        setup = setup or _HandlerSetup()
        if setup.formatter is not None:
            self.handler.setFormatter(setup.formatter)
        if setup.handler_level is not None:
            self.handler.setLevel(setup.handler_level)

        logger = FemtoLogger(request.logger_name)
        if request.logger_level is not None:
            logger.set_level(request.logger_level)
        logger.add_handler(self.adapter)
        logger.log(
            request.level,
            request.message,
            stack_info=request.stack_info,
            exc_info=request.exc_info,
        )
        del logger

        return self.stream.getvalue()


@pytest.fixture
def probe() -> AdapterProbe:
    """Return an adapter wrapping a StreamHandler over a fresh StringIO."""
    stream = io.StringIO()
    handler = logging.StreamHandler(stream)
    return AdapterProbe(stream, handler, StdlibHandlerAdapter(handler))


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
            _LogRequest("myapp", "INFO", "hello world"),
            _HandlerSetup(
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
            _LevelCase("ERROR", "ERROR", "failure"),
            _LevelCase("DEBUG", "DEBUG", "trace detail", "DEBUG", logging.DEBUG),
            _LevelCase("CRITICAL", "CRITICAL", "fatal"),
            _LevelCase("WARN", "WARNING", "caution"),
            _LevelCase("TRACE", "TRACE", "lowest", "TRACE", TRACE_LEVEL_NUM),
        ],
        ids=["error", "debug", "critical", "warn", "trace"],
    )
    def test_level_mapped(probe: AdapterProbe, case: _LevelCase) -> None:
        """Each femtologging level should map to its stdlib equivalent."""
        output = probe.capture(
            _LogRequest("app", case.level, case.message, case.logger_level),
            _HandlerSetup(
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


class TestHandlerLevelThreshold:
    """Verify the wrapped handler's level gates dispatch."""

    @staticmethod
    @pytest.mark.parametrize(
        "case",
        [
            _HandlerLevelCase("TRACE", "lowest", None),
            _HandlerLevelCase("DEBUG", "detail", None),
            _HandlerLevelCase("INFO", "note", None),
            _HandlerLevelCase("WARNING", "near miss", None),
            _HandlerLevelCase("ERROR", "failure", "ERROR"),
            _HandlerLevelCase("CRITICAL", "fatal", "CRITICAL"),
        ],
        ids=["trace", "debug", "info", "warning", "error", "critical"],
    )
    def test_error_handler_rejects_below_threshold(
        probe: AdapterProbe,
        case: _HandlerLevelCase,
    ) -> None:
        """An ERROR handler should emit only ERROR and CRITICAL records.

        The femtologging logger is left at ``"TRACE"``, the most permissive
        threshold available, so any record that fails to appear can only have
        been rejected by the wrapped handler's own level.
        """
        output = probe.capture(
            _LogRequest("app", case.level, case.message, logger_level="TRACE"),
            _HandlerSetup(
                formatter=logging.Formatter("%(levelname)s %(message)s"),
                handler_level=logging.ERROR,
            ),
        )
        expected = (
            ""
            if case.expected_levelname is None
            else f"{case.expected_levelname} {case.message}\n"
        )
        assert output == expected, (
            f"handler at ERROR gave {output!r} for level {case.level!r}, "
            f"expected {expected!r}"
        )

    @staticmethod
    @pytest.mark.parametrize(
        "level",
        ["TRACE", "DEBUG", "INFO"],
        ids=["trace", "debug", "info"],
    )
    def test_notset_handler_accepts_every_level(probe: AdapterProbe, level: str) -> None:
        """A NOTSET handler should accept records at every mapped level.

        ``logging.NOTSET`` is numeric zero, which is falsy. A truthiness test
        in place of the documented comparison would treat it as an
        impossibly high threshold and silently drop everything, so TRACE -- the
        lowest mapped numeric level -- is the case that exposes that mistake.
        """
        output = probe.capture(
            _LogRequest("app", level, "accepted", logger_level="TRACE"),
            _HandlerSetup(
                formatter=logging.Formatter("%(message)s"),
                handler_level=logging.NOTSET,
            ),
        )
        assert output == "accepted\n", (
            f"NOTSET handler dropped a {level} record: {output!r}"
        )


class TestHandleRecordFilterAndLockContract:
    """Verify the level gate leaves the filter-and-lock path intact."""

    @staticmethod
    def test_handle_and_filter_counts_by_level() -> None:
        """Rejected records skip the handler; accepted ones run it once."""
        handler = _CountingHandler()
        handler.setLevel(logging.ERROR)
        counting_filter = _CountingFilter()
        handler.addFilter(counting_filter)
        adapter = StdlibHandlerAdapter(handler)

        _dispatch(adapter, "DEBUG", "suppressed")
        assert handler.handle_calls == 0, (
            f"rejected record still entered handle(): {handler.handle_calls} calls"
        )
        assert counting_filter.calls == 0, (
            f"rejected record still reached the filter: {counting_filter.calls} calls"
        )
        assert handler.records == [], (
            f"rejected record was emitted: {[r.getMessage() for r in handler.records]!r}"
        )

        _dispatch(adapter, "ERROR", "accepted")
        assert handler.handle_calls == 1, (
            f"accepted record should enter handle() once: {handler.handle_calls}"
        )
        assert counting_filter.calls == 1, (
            f"accepted record should consult the filter once: {counting_filter.calls}"
        )
        assert [record.getMessage() for record in handler.records] == ["accepted"], (
            f"unexpected emitted records: "
            f"{[r.getMessage() for r in handler.records]!r}"
        )

    @staticmethod
    def test_filter_rejection_still_runs_handle() -> None:
        """A record above the level is still subject to the handler's filters."""
        handler = _CountingHandler()
        handler.setLevel(logging.ERROR)
        rejecting_filter = _RejectingFilter()
        handler.addFilter(rejecting_filter)
        adapter = StdlibHandlerAdapter(handler)

        _dispatch(adapter, "CRITICAL", "filtered out")

        assert handler.handle_calls == 1, (
            f"an accepted record should still enter handle(): {handler.handle_calls}"
        )
        assert rejecting_filter.calls == 1, (
            f"an accepted record should consult the filter once: "
            f"{rejecting_filter.calls}"
        )
        assert handler.records == [], (
            f"a filter-rejected record must not be emitted: "
            f"{[r.getMessage() for r in handler.records]!r}"
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
                _LogRequest("app", "ERROR", "caught", exc_info=True),
                _HandlerSetup(formatter=logging.Formatter("%(message)s\n%(exc_text)s")),
            )
        assert "caught" in output, f"expected 'caught' in output: {output!r}"
        assert "ValueError" in output, f"expected 'ValueError' in output: {output!r}"

    @staticmethod
    def test_stack_info_forwarded(probe: AdapterProbe) -> None:
        """Stack trace payload should appear as stack_info on the LogRecord."""
        output = probe.capture(
            _LogRequest("app", "INFO", "trace", stack_info=True),
            _HandlerSetup(formatter=logging.Formatter("%(message)s")),
        )
        assert "trace" in output, f"expected 'trace' in output: {output!r}"
        # stdlib Formatter appends stack_info after the message
        assert "Stack (most recent call last)" in output, (
            f"expected stack trace header in output: {output!r}"
        )
