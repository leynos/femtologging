"""Shared record and adapter vocabulary for the ``StdlibHandlerAdapter`` suites.

The adapter's tests are split across construction and dispatch
(``tests/test_stdlib_adapter.py``), the wrapped handler's level threshold
(``tests/test_stdlib_adapter_levels.py``), and ``LogRecord`` population and
delegation (``tests/test_stdlib_adapter_records.py``).  Everything all three
need lives here, so a change to the record schema is made once rather than kept
consistent across three files by hand. Handler and filter probes are defined
in ``tests.stdlib_adapter_handler_support`` and re-exported here.
"""

from __future__ import annotations

import dataclasses
import io
import logging

from femtologging import FemtoLogger, StdlibHandlerAdapter
from tests.stdlib_adapter_handler_support import (
    CapturingHandler,
    CountingFilter,
    CountingHandler,
    RejectingFilter,
    emitted_messages,
)

__all__ = [
    "AdapterProbe",
    "CapturingHandler",
    "CountingFilter",
    "CountingHandler",
    "HandlerLevelCase",
    "HandlerSetup",
    "LevelCase",
    "LogRequest",
    "RejectingFilter",
    "build_adapter_probe",
    "dispatch",
    "emitted_messages",
    "femto_logger",
]

# -- Record requests and handler setup ------------------------------------


@dataclasses.dataclass(frozen=True, slots=True)
class LevelCase:
    """A single level-mapping test case.

    Attributes
    ----------
    level : str
        Femtologging level to send through the adapter.
    expected_level : str
        Level name expected in the captured output.
    message : str
        Message text to log.
    logger_level : str or None
        Logger threshold to set, or ``None`` to keep its default.
    handler_level : int or None
        Wrapped-handler threshold to set, or ``None`` to keep its default.

    """

    level: str
    expected_level: str
    message: str
    logger_level: str | None = None
    handler_level: int | None = None


@dataclasses.dataclass(frozen=True, slots=True)
class LogRequest:
    """One log emission routed through the adapter under test.

    Attributes
    ----------
    logger_name : str
        Name of the femtologging logger to use.
    level : str
        Femtologging severity level to emit.
    message : str
        Message text to log.
    logger_level : str or None
        Logger threshold to set, or ``None`` to keep its default.
    stack_info : bool
        Whether to capture stack information for the record.
    exc_info : bool
        Whether to capture the active exception for the record.

    """

    logger_name: str
    level: str
    message: str
    logger_level: str | None = None
    stack_info: bool = False
    exc_info: bool = False


@dataclasses.dataclass(frozen=True, slots=True)
class HandlerSetup:
    """Optional stdlib handler configuration applied before logging.

    Attributes
    ----------
    formatter : logging.Formatter or None
        Formatter to install before the request is dispatched.
    handler_level : int or None
        Handler threshold to set, or ``None`` to keep the current level.

    """

    formatter: logging.Formatter | None = None
    handler_level: int | None = None


@dataclasses.dataclass(frozen=True, slots=True)
class HandlerLevelCase:
    """One record level routed through a wrapped handler at a fixed level.

    Attributes
    ----------
    level : str
        Femtologging severity level to emit.
    message : str
        Message text to log.
    expected_levelname : str or None
        Expected stdlib level name, or ``None`` when the handler rejects it.

    """

    level: str
    message: str
    expected_levelname: str | None


@dataclasses.dataclass(frozen=True, slots=True)
class AdapterProbe:
    """A StringIO sink, its stdlib handler, and the adapter wrapping it.

    Attributes
    ----------
    stream : io.StringIO
        In-memory stream receiving the handler's formatted output.
    handler : logging.StreamHandler
        Stdlib stream handler writing to ``stream``.
    adapter : StdlibHandlerAdapter
        Adapter wrapping ``handler`` for use with femtologging.

    """

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
