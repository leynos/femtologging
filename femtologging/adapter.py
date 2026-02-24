"""Adapter bridging stdlib ``logging.Handler`` subclasses to femtologging.

Femtologging dispatches to handlers via ``handle_record(record)`` where
*record* is a plain dictionary.  Stdlib handlers expect ``emit(LogRecord)``.
``StdlibHandlerAdapter`` translates between the two interfaces so that any
``logging.Handler`` subclass can be attached to a ``FemtoLogger``.
"""

from __future__ import annotations

import logging
import typing as typ
import warnings

# -- Record schema TypedDicts ---------------------------------------------


class Frame(typ.TypedDict):
    """A single stack frame in an exception or stack payload."""

    filename: str
    lineno: int
    function: str
    source_line: typ.NotRequired[str]


class Metadata(typ.TypedDict, total=False):
    """The ``metadata`` sub-dictionary of a femtologging record."""

    filename: str
    line_number: int
    timestamp: float
    thread_name: str
    thread_id: int | str
    key_values: dict[str, object]


class ExcInfo(typ.TypedDict, total=False):
    """The ``exc_info`` sub-dictionary of a femtologging record."""

    type_name: str
    message: str
    frames: list[Frame]


class StackInfo(typ.TypedDict, total=False):
    """The ``stack_info`` sub-dictionary of a femtologging record."""

    frames: list[Frame]


class FemtoRecord(typ.TypedDict, total=False):
    """Schema for the record dict passed to ``handle_record``."""

    logger: str
    level: str
    levelno: int
    message: str
    metadata: Metadata
    exc_info: ExcInfo
    stack_info: StackInfo


# -- Femtologging level (0-5) -> stdlib level (5-50) ---------------------

TRACE_LEVEL_NUM = 5
logging.addLevelName(TRACE_LEVEL_NUM, "TRACE")

_FEMTO_TO_STDLIB_LEVEL: dict[int, int] = {
    0: TRACE_LEVEL_NUM,  # TRACE  -> TRACE
    1: 10,  # DEBUG  -> DEBUG
    2: 20,  # INFO   -> INFO
    3: 30,  # WARN   -> WARNING
    4: 40,  # ERROR  -> ERROR
    5: 50,  # CRITICAL -> CRITICAL
}

_FEMTO_LEVEL_NAMES: dict[str, int] = {
    "TRACE": TRACE_LEVEL_NUM,
    "DEBUG": logging.DEBUG,
    "INFO": logging.INFO,
    "WARN": logging.WARNING,
    "WARNING": logging.WARNING,
    "ERROR": logging.ERROR,
    "CRITICAL": logging.CRITICAL,
}


def _stdlib_levelno(record: FemtoRecord) -> int:
    """Derive the stdlib numeric level from a femtologging record.

    Tries the numeric ``levelno`` first, falling back to the string
    ``level`` name.  Returns ``logging.WARNING`` if neither can be
    resolved.
    """
    levelno = record.get("levelno")
    if isinstance(levelno, int) and levelno in _FEMTO_TO_STDLIB_LEVEL:
        return _FEMTO_TO_STDLIB_LEVEL[levelno]

    name = record.get("level")
    if isinstance(name, str) and name in _FEMTO_LEVEL_NAMES:
        return _FEMTO_LEVEL_NAMES[name]

    return logging.WARNING


def _format_frames(frames: list[Frame]) -> list[str]:
    """Render a list of stack frame dicts as human-readable text lines.

    Parameters
    ----------
    frames
        Frame dictionaries with ``filename``, ``lineno``, ``function``,
        and optionally ``source_line`` keys.

    Returns
    -------
    list[str]
        Lines describing each frame.

    """
    lines: list[str] = []
    for frame in frames:
        filename = frame.get("filename", "<unknown>")
        lineno = frame.get("lineno", "?")
        function = frame.get("function", "<unknown>")
        lines.append(f'  File "{filename}", line {lineno}, in {function}')
        source = frame.get("source_line")
        if source:
            lines.append(f"    {source}")
    return lines


def _format_exc_text(exc_info: ExcInfo) -> str:
    """Render a femtologging exception payload as human-readable text.

    Parameters
    ----------
    exc_info
        The ``exc_info`` sub-dictionary from a femtologging record.

    Returns
    -------
    str
        A multi-line traceback string similar to stdlib's formatting.

    Examples
    --------
    >>> payload = {"type_name": "ValueError", "message": "bad",
    ...            "frames": [{"filename": "a.py", "lineno": 1,
    ...                        "function": "f"}]}
    >>> text = _format_exc_text(payload)
    >>> "ValueError: bad" in text
    True

    """
    lines: list[str] = ["Traceback (most recent call last):"]
    lines.extend(_format_frames(exc_info.get("frames", [])))

    type_name = exc_info.get("type_name", "Exception")
    message = exc_info.get("message", "")
    lines.append(f"{type_name}: {message}" if message else type_name)

    return "\n".join(lines)


def _format_stack_text(stack_info: StackInfo) -> str:
    """Render a femtologging stack payload as human-readable text.

    Parameters
    ----------
    stack_info
        The ``stack_info`` sub-dictionary from a femtologging record.

    Returns
    -------
    str
        A multi-line stack trace string.

    """
    lines: list[str] = ["Stack (most recent call last):"]
    lines.extend(_format_frames(stack_info.get("frames", [])))
    return "\n".join(lines)


def _populate_thread_info(
    log_record: logging.LogRecord,
    metadata: Metadata,
) -> None:
    """Set thread-related attributes on a LogRecord from metadata.

    Parameters
    ----------
    log_record
        The stdlib LogRecord to populate.
    metadata
        The femtologging metadata sub-dictionary.

    """
    thread_name = metadata.get("thread_name")
    if thread_name is not None:
        log_record.threadName = str(thread_name)

    thread_id = metadata.get("thread_id")
    if thread_id is None:
        return

    # femtologging formats thread_id as a Rust debug string; try to
    # extract a numeric value but fall back gracefully.
    try:
        tid_str = str(thread_id).strip()
        # Rust formats ThreadId as "ThreadId(N)"
        if tid_str.startswith("ThreadId(") and tid_str.endswith(")"):
            tid_str = tid_str[9:-1]
        log_record.thread = int(tid_str)
    except (ValueError, TypeError):
        pass


class StdlibHandlerAdapter:
    """Wrap a stdlib ``logging.Handler`` for use with femtologging.

    The adapter implements the ``handle_record`` interface expected by
    femtologging's handler pipeline.  When femtologging dispatches a
    record, the adapter constructs a ``logging.LogRecord`` and calls
    the wrapped handler's ``handle()`` method so that handler-level
    filtering, attached filters, and I/O locking are applied.

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
        self._handler = handler

    # -- femtologging handler protocol ------------------------------------

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback required by femtologging's handler validation.

        ``StdlibHandlerAdapter`` always exposes ``handle_record``, so
        this method should never be called at runtime.  It exists solely
        to satisfy the ``add_handler()`` check for a callable ``handle``
        attribute.  A ``RuntimeWarning`` is emitted to surface
        accidental use.
        """
        warnings.warn(
            "StdlibHandlerAdapter.handle() was called directly; "
            "this is a no-op — handle_record() should be used instead",
            RuntimeWarning,
            stacklevel=2,
        )

    def handle_record(self, record: FemtoRecord) -> None:
        """Translate a femtologging record dict and dispatch via the stdlib handler.

        The constructed ``LogRecord`` is passed through
        ``self._handler.handle()`` rather than ``emit()`` directly, so
        handler-level filtering, any attached ``logging.Filter``
        instances, and the handler's I/O lock are all applied.

        Parameters
        ----------
        record
            The femtologging record dictionary.  See the users guide
            for the full schema.

        """
        log_record = _make_log_record(record)
        self._handler.handle(log_record)

    # -- delegation -------------------------------------------------------

    def flush(self) -> None:
        """Flush the wrapped handler."""
        self._handler.flush()

    def close(self) -> None:
        """Close the wrapped handler."""
        self._handler.close()


def _make_log_record(record: FemtoRecord) -> logging.LogRecord:
    """Build a ``logging.LogRecord`` from a femtologging record dict.

    Parameters
    ----------
    record
        The femtologging record dictionary.

    Returns
    -------
    logging.LogRecord
        A stdlib-compatible log record populated with available
        fields from the femtologging record.

    """
    metadata: Metadata = record.get("metadata", {})

    log_record = logging.LogRecord(
        name=record.get("logger", "femtologging"),
        level=_stdlib_levelno(record),
        pathname=metadata.get("filename", "<unknown>"),
        lineno=metadata.get("line_number", 0),
        msg=record.get("message", ""),
        args=(),
        exc_info=None,
    )

    log_record.levelname = logging.getLevelName(log_record.levelno)

    timestamp = metadata.get("timestamp")
    if isinstance(timestamp, (int, float)):
        log_record.created = float(timestamp)
        log_record.msecs = (log_record.created - int(log_record.created)) * 1000.0

    _populate_thread_info(log_record, metadata)

    exc_info = record.get("exc_info")
    if isinstance(exc_info, dict):
        log_record.exc_text = _format_exc_text(exc_info)

    stack_info = record.get("stack_info")
    if isinstance(stack_info, dict):
        log_record.stack_info = _format_stack_text(stack_info)

    # Propagate custom key-value pairs so stdlib formatters/filters
    # can reference them (e.g. ``%(request_id)s``).
    key_values = metadata.get("key_values")
    if isinstance(key_values, dict):
        for key, value in key_values.items():
            if isinstance(key, str):
                log_record.__dict__[key] = value

    return log_record
