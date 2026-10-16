"""Shared record and handler vocabulary for the ``StdlibHandlerAdapter`` suites.

The adapter's tests are split across construction and dispatch
(``tests/test_stdlib_adapter.py``), the wrapped handler's level threshold
(``tests/test_stdlib_adapter_levels.py``), and ``LogRecord`` population and
delegation (``tests/test_stdlib_adapter_records.py``).  Everything all three
need lives here, so a change to the record schema or to a counting helper is
made once rather than kept consistent across three files by hand.
"""

from __future__ import annotations

import dataclasses
import io
import logging
import typing as typ

from femtologging import FemtoLogger, StdlibHandlerAdapter

# -- Record requests and handler setup ------------------------------------


@dataclasses.dataclass(frozen=True, slots=True)
class LevelCase:
    """A single level-mapping test case."""

    level: str
    expected_level: str
    message: str
    logger_level: str | None = None
    handler_level: int | None = None


@dataclasses.dataclass(frozen=True, slots=True)
class LogRequest:
    """One log emission routed through the adapter under test."""

    logger_name: str
    level: str
    message: str
    logger_level: str | None = None
    stack_info: bool = False
    exc_info: bool = False


@dataclasses.dataclass(frozen=True, slots=True)
class HandlerSetup:
    """Optional stdlib handler configuration applied before logging."""

    formatter: logging.Formatter | None = None
    handler_level: int | None = None


@dataclasses.dataclass(frozen=True, slots=True)
class HandlerLevelCase:
    """One record level routed through a wrapped handler at a fixed level."""

    level: str
    message: str
    expected_levelname: str | None


@dataclasses.dataclass(frozen=True, slots=True)
class AdapterProbe:
    """A StringIO sink, its stdlib handler, and the adapter wrapping it."""

    stream: io.StringIO
    handler: logging.StreamHandler[io.StringIO]
    adapter: StdlibHandlerAdapter

    def capture(self, request: LogRequest, setup: HandlerSetup | None = None) -> str:
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
        >>> probe.capture(LogRequest("doctest.adapter", "INFO", "hi")).strip()
        'hi'

        """
        setup = setup or HandlerSetup()
        if setup.formatter is not None:
            self.handler.setFormatter(setup.formatter)
        if setup.handler_level is not None:
            self.handler.setLevel(setup.handler_level)

        logger = femto_logger(request)
        logger.add_handler(self.adapter)
        logger.log(
            request.level,
            request.message,
            stack_info=request.stack_info,
            exc_info=request.exc_info,
        )
        del logger

        return self.stream.getvalue()


def build_adapter_probe() -> AdapterProbe:
    """Return an adapter wrapping a ``StreamHandler`` over a fresh ``StringIO``.

    Both adapter suites need the same fixture; building it here keeps the
    formatter-free, ``NOTSET``-level starting state identical between them, so a
    verdict in one suite cannot differ because of how its probe was assembled.

    Returns
    -------
    AdapterProbe
        A probe whose stream is empty and whose handler is at ``NOTSET``.

    Examples
    --------
    >>> probe = build_adapter_probe()
    >>> probe.stream.getvalue()
    ''

    """
    stream = io.StringIO()
    handler = logging.StreamHandler(stream)
    return AdapterProbe(stream, handler, StdlibHandlerAdapter(handler))


def femto_logger(request: LogRequest) -> FemtoLogger:
    """Return the femtologging logger *request* asks for.

    Parameters
    ----------
    request
        The record request whose logger name and threshold are used.

    Returns
    -------
    FemtoLogger
        A logger named after the request, at its requested threshold.

    Examples
    --------
    >>> femto_logger(LogRequest("app.sub", "INFO", "hi", "DEBUG")).level
    'DEBUG'
    >>> femto_logger(LogRequest("app.sub", "INFO", "hi")).isEnabledFor("TRACE")
    False

    """
    logger = FemtoLogger(request.logger_name)
    if request.logger_level is not None:
        logger.set_level(request.logger_level)
    return logger


# -- Counting and capturing handlers --------------------------------------


class CapturingHandler(logging.Handler):
    """Handler that captures emitted LogRecords for test inspection."""

    def __init__(self) -> None:
        """Initialise with an empty records list."""
        super().__init__()
        self.records: list[logging.LogRecord] = []

    @typ.override
    def emit(self, record: logging.LogRecord) -> None:
        self.records.append(record)


class CountingHandler(logging.Handler):
    """Handler that counts ``handle()`` calls and captures emitted records.

    ``Handler.handle()`` is overridden rather than ``emit()`` so a test can
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


class CountingFilter(logging.Filter):
    """Filter that counts how often the handler consults it."""

    def __init__(self) -> None:
        """Initialise with a zero filter-call count."""
        super().__init__()
        self.calls = 0

    @typ.override
    def filter(self, record: logging.LogRecord) -> bool:
        self.calls += 1
        return True


class RejectingFilter(logging.Filter):
    """Filter that rejects every record and counts the consultations."""

    def __init__(self) -> None:
        """Initialise with a zero filter-call count."""
        super().__init__()
        self.calls = 0

    @typ.override
    def filter(self, record: logging.LogRecord) -> bool:
        self.calls += 1
        return False


def dispatch(adapter: StdlibHandlerAdapter, level: str, message: str) -> None:
    """Send one record straight to *adapter* without a worker thread.

    Calling ``handle_record()`` directly keeps counting assertions
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
    >>> handler = CountingHandler()
    >>> handler.setLevel(logging.ERROR)
    >>> dispatch(StdlibHandlerAdapter(handler), "ERROR", "boom")
    >>> [record.getMessage() for record in handler.records]
    ['boom']

    """
    adapter.handle_record({"logger": "app", "level": level, "message": message})


def emitted_messages(handler: CapturingHandler | CountingHandler) -> list[str]:
    """Return the messages of every record *handler* emitted.

    Returns
    -------
    list[str]
        One message per emitted record, in emission order.

    Examples
    --------
    >>> handler = CapturingHandler()
    >>> emitted_messages(handler)
    []

    """
    return [record.getMessage() for record in handler.records]
