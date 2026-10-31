"""Translate femtologging record dicts into stdlib ``logging.LogRecord`` objects.

Femtologging hands its handlers plain dictionaries; stdlib handlers expect
``LogRecord`` instances.  This module owns the schema for those dictionaries,
the femtologging-to-stdlib level mapping, and the translation itself.  The
adapter in :mod:`femtologging.adapter` supplies the handler-protocol plumbing
and the severity gate that decides whether a translated record is dispatched.
"""

from __future__ import annotations

import logging
import threading
import time
import types
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

# -- Record schema TypedDicts ---------------------------------------------


class Frame(typ.TypedDict):
    """A single stack frame in an exception or stack payload.

    Attributes
    ----------
    filename : str
        Source file that contains the frame.
    lineno : int
        One-based source line number for the frame.
    function : str
        Name of the function executing in the frame.
    source_line : str, optional
        Source text for the frame when it was available during capture.

    """

    filename: str
    lineno: int
    function: str
    source_line: typ.NotRequired[str]


class Metadata(typ.TypedDict, total=False):
    """The optional ``metadata`` sub-dictionary of a femtologging record.

    Attributes
    ----------
    filename : str, optional
        Source file that produced the record.
    line_number : int, optional
        One-based source line number that produced the record.
    timestamp : float, optional
        Record creation time in seconds since the Unix epoch.
    thread_name : str, optional
        Name of the thread that produced the record.
    thread_id : int or str, optional
        Identifier of the thread that produced the record.
    key_values : dict of str to object, optional
        Structured context values attached to the record.

    """

    filename: str
    line_number: int
    timestamp: float
    thread_name: str
    thread_id: int | str
    key_values: dict[str, object]


class ExcInfo(typ.TypedDict, total=False):
    """The optional ``exc_info`` sub-dictionary of a femtologging record.

    Attributes
    ----------
    schema_version : int
        Required version used to interpret the exception payload.
    type_name : str, optional
        Name of the captured exception type.
    message : str, optional
        Text of the captured exception message.
    frames : list of Frame, optional
        Stack frames captured for the exception.

    """

    schema_version: typ.Required[int]
    type_name: str
    message: str
    frames: list[Frame]


class StackInfo(typ.TypedDict, total=False):
    """The optional ``stack_info`` sub-dictionary of a femtologging record.

    Attributes
    ----------
    frames : list of Frame, optional
        Stack frames captured for the current call stack.

    """

    frames: list[Frame]


class FemtoRecord(typ.TypedDict, total=False):
    """Schema for the record dictionary passed to ``handle_record``.

    All fields are optional in the dictionary type so partial records can be
    translated using the adapter's documented fallbacks.

    Attributes
    ----------
    logger : str, optional
        Name of the femtologging logger that produced the record.
    level : str, optional
        Name of the femtologging severity level.
    levelno : int, optional
        Numeric femtologging severity, from 0 (TRACE) through 5 (CRITICAL).
    message : str, optional
        Formatted message text for the record.
    metadata : Metadata, optional
        Source, timestamp, thread, and structured context information.
    exc_info : ExcInfo, optional
        Structured exception details when exception capture was requested.
    stack_info : StackInfo, optional
        Structured call-stack details when stack capture was requested.

    """

    logger: str
    level: str
    levelno: int
    message: str
    metadata: Metadata
    exc_info: ExcInfo
    stack_info: StackInfo


# -- Femtologging level (0-5) -> stdlib level (5-50) ---------------------

TRACE_LEVEL_NUM = 5
_trace_registered = False
_trace_register_lock = threading.Lock()


def _ensure_trace_level() -> None:
    """Register the TRACE level name with stdlib logging if not already done."""
    global _trace_registered
    if _trace_registered:
        return
    with _trace_register_lock:
        if not _trace_registered:
            logging.addLevelName(TRACE_LEVEL_NUM, "TRACE")
            _trace_registered = True


_FEMTO_TO_STDLIB_LEVEL: types.MappingProxyType[int, int] = types.MappingProxyType({
    0: TRACE_LEVEL_NUM,  # TRACE  -> TRACE
    1: 10,  # DEBUG  -> DEBUG
    2: 20,  # INFO   -> INFO
    3: 30,  # WARN   -> WARNING
    4: 40,  # ERROR  -> ERROR
    5: 50,  # CRITICAL -> CRITICAL
})

_FEMTO_LEVEL_NAMES: types.MappingProxyType[str, int] = types.MappingProxyType({
    "TRACE": TRACE_LEVEL_NUM,
    "DEBUG": logging.DEBUG,
    "INFO": logging.INFO,
    "WARN": logging.WARNING,
    "WARNING": logging.WARNING,
    "ERROR": logging.ERROR,
    "CRITICAL": logging.CRITICAL,
})


def _stdlib_levelno(record: FemtoRecord) -> int:
    """Derive the stdlib numeric level from a femtologging record.

    Returns
    -------
    int
        The mapped level, preferring ``levelno``, then ``level``, and finally
        ``logging.WARNING`` when neither resolves.
    """
    levelno = record.get("levelno")
    if isinstance(levelno, int) and levelno in _FEMTO_TO_STDLIB_LEVEL:
        return _FEMTO_TO_STDLIB_LEVEL[levelno]

    name = record.get("level")
    if isinstance(name, str) and name in _FEMTO_LEVEL_NAMES:
        return _FEMTO_LEVEL_NAMES[name]

    return logging.WARNING


def _format_frames(frames: list[Frame]) -> cabc.Iterator[str]:
    """Render a list of stack frame dicts as human-readable text lines.

    Parameters
    ----------
    frames
        Frame dictionaries with ``filename``, ``lineno``, ``function``,
        and optionally ``source_line`` keys.

    Yields
    ------
    str
        Lines describing each frame.

    """
    for frame in frames:
        filename = frame.get("filename", "<unknown>")
        lineno = frame.get("lineno", 0)
        function = frame.get("function", "<unknown>")
        yield f'  File "{filename}", line {lineno}, in {function}'
        source = frame.get("source_line")
        if source:
            yield f"    {source}"


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

    # Extract the numeric value from femtologging's Rust debug string.
    try:
        tid_str = str(thread_id).strip()
        # Rust formats ThreadId as "ThreadId(N)"
        if tid_str.startswith("ThreadId(") and tid_str.endswith(")"):
            tid_str = tid_str[9:-1]
        log_record.thread = int(tid_str)
    except (ValueError, TypeError):
        pass


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
        # logging._startTime is set at logging import; use getattr for
        # type-checker compatibility with the private attribute.
        start_time: float = getattr(logging, "_startTime", time.time())
        log_record.relativeCreated = (log_record.created - start_time) * 1000.0

    _populate_thread_info(log_record, metadata)

    exc_info = record.get("exc_info")
    if isinstance(exc_info, dict):
        log_record.exc_text = _format_exc_text(exc_info)

    stack_info = record.get("stack_info")
    if isinstance(stack_info, dict):
        log_record.stack_info = _format_stack_text(stack_info)

    # Propagate custom key-value pairs so stdlib formatters/filters
    # can reference them (e.g. ``%(request_id)s``).  Skip keys that
    # already exist on the LogRecord to avoid overwriting core attrs.
    key_values = metadata.get("key_values")
    if isinstance(key_values, dict):
        for key, value in key_values.items():
            if isinstance(key, str) and key not in log_record.__dict__:
                log_record.__dict__[key] = value

    return log_record
