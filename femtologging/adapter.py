"""Adapter bridging stdlib ``logging.Handler`` subclasses to femtologging.

Femtologging passes plain-dict records to ``handle_record``; this adapter
translates them into ``LogRecord`` instances, then applies the wrapped
handler's severity threshold before dispatching eligible records.  The record
schema and the translation itself live in :mod:`femtologging._adapter_record`
and are re-exported here, so ``femtologging.adapter`` remains the single import
site for everything the adapter family exposes.
"""

from __future__ import annotations

import logging
import warnings

from ._adapter_record import (
    TRACE_LEVEL_NUM,
    ExcInfo,
    FemtoRecord,
    Frame,
    Metadata,
    StackInfo,
    _ensure_trace_level,
    _make_log_record,
)

__all__ = [
    "TRACE_LEVEL_NUM",
    "ExcInfo",
    "FemtoRecord",
    "Frame",
    "Metadata",
    "StackInfo",
    "StdlibHandlerAdapter",
]


class StdlibHandlerAdapter:
    """Wrap a stdlib ``logging.Handler`` for use with femtologging.

    The adapter implements the ``handle_record`` interface expected by
    femtologging's handler pipeline.  When femtologging dispatches a
    record, the adapter constructs a ``logging.LogRecord``, discards it
    when its level falls below the wrapped handler's configured level,
    and otherwise calls the wrapped handler's ``handle()`` method so
    that attached filters and stdlib locking semantics are applied. Filters
    may still reject a record that passes the level check; the handler acquires
    its I/O lock only when its filters accept the record.

    Two thresholds therefore apply.  The femtologging logger's own level
    decides whether a record is dispatched at all.  The wrapped stdlib
    handler's level, set with ``logging.Handler.setLevel()``, then decides
    whether the record enters that handler's ``handle()`` path. Its filters
    can further reject the record before emission.

    Femtologging dispatches asynchronously on a dedicated worker thread. It
    captures the producer's ``contextvars`` context before queueing and runs the
    wrapped handler's entire ``handle()`` call (filters, formatters, and
    ``emit()``) inside it. Other thread-local state, such as ``threading.local``,
    remains worker-local.

    Parameters
    ----------
    handler
        A ``logging.Handler`` (or subclass) instance.

    Raises
    ------
    TypeError
        If *handler* is not an instance of ``logging.Handler``.

    Examples
    --------
    >>> import logging, io
    >>> stream = io.StringIO()
    >>> stdlib_handler = logging.StreamHandler(stream)
    >>> adapter = StdlibHandlerAdapter(stdlib_handler)

    Attach the adapter to a femtologging logger via
    ``logger.add_handler(adapter)``.

    """

    def __init__(self, handler: logging.Handler) -> None:
        """Wrap *handler* for use with femtologging.

        Parameters
        ----------
        handler
            A ``logging.Handler`` (or subclass) instance.

        Raises
        ------
        TypeError
            If *handler* is not a ``logging.Handler``.

        """
        if not isinstance(handler, logging.Handler):
            msg = f"expected a logging.Handler instance, got {type(handler).__name__}"
            raise TypeError(msg)
        _ensure_trace_level()
        self._handler = handler

    # -- femtologging handler protocol ------------------------------------

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback required by femtologging's handler validation.

        ``StdlibHandlerAdapter`` always exposes ``handle_record``, so this
        method should never be called at runtime.  It exists solely to satisfy
        the ``add_handler()`` check for a callable ``handle`` attribute.  A
        ``RuntimeWarning`` is emitted to surface accidental use.
        """
        warnings.warn(
            "StdlibHandlerAdapter.handle() was called directly; "
            "this is a no-op — handle_record() should be used instead",
            RuntimeWarning,
            stacklevel=2,
        )

    def handle_record(self, record: FemtoRecord) -> None:
        """Translate a femtologging record dict and dispatch via the stdlib handler.

        Two independent thresholds decide whether a record enters the wrapped
        handler. Femtologging's own logger threshold is applied before dispatch
        and is not this method's concern. This method first constructs a
        ``LogRecord``, then applies the wrapped handler's ``level``, mirroring
        ``logging.Logger.callHandlers()``: a record whose mapped stdlib level is
        below that level is discarded without calling ``handle()``. Because the
        level is read on every call, a ``setLevel()`` on the wrapped handler
        affects the next record.

        A record that passes the level check is passed through
        ``self._handler.handle()`` exactly once rather than ``emit()`` directly.
        ``Handler.handle()`` applies attached ``logging.Filter`` instances once
        and acquires the I/O lock only when those filters accept the record.
        Filters may therefore reject a level-accepted record before emission.
        ``logging.NOTSET`` needs no special case: it is numeric zero, so every
        mapped level clears the comparison.

        Parameters
        ----------
        record
            The femtologging record dictionary.  See the users guide
            for the full schema.

        """
        log_record = _make_log_record(record)
        if log_record.levelno >= self._handler.level:
            self._handler.handle(log_record)

    # -- delegation -------------------------------------------------------
    # The forwarders below are part of the femtologging handler protocol, so
    # they must stay callable on the adapter; there is no call site to inline.
    # pylint: disable=unknown-option-value, trivial-attribute-wrapper
    def flush(self) -> None:
        """Flush the wrapped handler."""
        self._handler.flush()

    def close(self) -> None:
        """Close the wrapped handler."""
        self._handler.close()
