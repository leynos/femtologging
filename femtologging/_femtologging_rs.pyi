import collections.abc as cabc
import types
import typing as typ
from typing import TypedDict  # noqa: ICN003 - explicit import required for stubs.

Callable = cabc.Callable
Mapping = cabc.Mapping
_Any = typ.Any
Final = typ.Final
Literal = typ.Literal
Self = typ.Self
Union = typ.Union
PolicyName = typ.Literal["drop", "block", "timeout"]

FemtoLevel: _Any
LevelName = Literal["TRACE", "DEBUG", "INFO", "WARN", "WARNING", "ERROR", "CRITICAL"]
LevelArg = Union[LevelName, FemtoLevel]

# Type alias for exc_info parameter
ExcInfo = Union[
    bool,
    BaseException,
    tuple[type[BaseException], BaseException, types.TracebackType | None],
    tuple[None, None, None],
    None,
]

class FemtoLogger:
    """A high-performance logger implemented in Rust."""

    def __init__(self, name: str) -> None: ...
    @property
    def parent(self) -> str | None: ...
    @property
    def level(self) -> str: ...
    @property
    def propagate(self) -> bool: ...
    def log(
        self,
        level: LevelArg,
        message: str,
        /,
        *,
        exc_info: ExcInfo = None,
        stack_info: bool = False,
    ) -> str | None:
        """Log a message at the given level.

        Parameters
        ----------
        level
            The log level (e.g., "INFO", "ERROR").
        message
            The log message.
        exc_info
            Optional exception information. Accepts:
            - ``True``: Capture the current exception via ``sys.exc_info()``.
            - An exception instance: Capture that exception's traceback.
            - A 3-tuple ``(type, value, traceback)``: Use directly.
        stack_info
            If ``True``, capture the current call stack.

        Returns
        -------
        str | None
            The formatted log message if the record passes level and filter
            checks, otherwise ``None``.

        """
        ...
    def set_level(self, level: LevelArg) -> None: ...
    def set_propagate(self, flag: bool) -> None: ...
    def add_handler(self, handler: object) -> None: ...
    def remove_handler(self, handler: object) -> bool: ...
    def clear_handlers(self) -> None: ...
    def clear_filters(self) -> None: ...
    def get_dropped(self) -> int: ...
    def flush_handlers(self) -> bool:
        """Flush all handlers attached to this logger.

        First waits up to 2 seconds for the internal worker thread to
        drain its queue, then calls ``flush()`` on every attached
        handler (each handler applies its own timeout).

        Returns
        -------
        bool
            ``True`` when the worker drains in time and every handler
            flush succeeds.
            ``False`` when the worker queue cannot be drained (channel
            closed or timeout exceeded) or any handler flush returns
            ``False``.

        """
        ...

FemtoHandler: _Any
FemtoStreamHandler: _Any
FemtoFileHandler: _Any
FemtoSocketHandler: _Any
FemtoHTTPHandler: _Any

class OverflowPolicy:
    @staticmethod
    def drop() -> OverflowPolicy: ...
    @staticmethod
    def block() -> OverflowPolicy: ...
    @staticmethod
    def timeout(timeout_ms: int) -> OverflowPolicy: ...

class HandlerOptions:
    capacity: int
    flush_interval: int
    policy: PolicyName
    max_bytes: int
    backup_count: int

    def __init__(
        self,
        capacity: int = ...,
        flush_interval: int = ...,
        policy: PolicyName = ...,
        rotation: tuple[int, int] | None = ...,
    ) -> None: ...

ROTATION_VALIDATION_MSG: Final[str]
EXCEPTION_SCHEMA_VERSION: Final[int]
StreamHandlerBuilder: _Any
SocketHandlerBuilder: _Any
FileHandlerBuilder: _Any

class FemtoRotatingFileHandler:
    def __init__(
        self,
        path: str,
        options: HandlerOptions | None = ...,
    ) -> None: ...
    @property
    def max_bytes(self) -> int: ...
    @property
    def backup_count(self) -> int: ...
    def handle(self, logger: str, level: LevelArg, message: str) -> None: ...
    def flush(self) -> bool:
        """Flush queued log records to disk without closing the handler.

        Uses a fixed 1-second timeout.

        Returns
        -------
        bool
            ``True`` when the worker acknowledges the flush within
            the timeout.
            ``False`` when the handler has already been closed, the
            internal channel to the worker has been dropped, or the
            worker does not acknowledge before the timeout elapses.

        """
        ...
    def close(self) -> None: ...

class RotatingFileHandlerBuilder:
    def __init__(self, path: str) -> None: ...
    def with_capacity(self, capacity: int) -> Self: ...
    def with_flush_after_records(self, interval: int) -> Self: ...
    def with_formatter(
        self, fmt: str | Callable[[Mapping[str, object]], str]
    ) -> Self: ...
    def with_max_bytes(self, max_bytes: int) -> Self: ...
    def with_backup_count(self, count: int) -> Self: ...
    def with_overflow_policy(self, policy: OverflowPolicy) -> Self: ...
    def as_dict(self) -> dict[str, object]: ...
    def build(self) -> FemtoRotatingFileHandler: ...

class BackoffConfigDict(TypedDict, total=False):
    """Configuration options for exponential backoff retry behaviour."""

    base_ms: int | None
    cap_ms: int | None
    reset_after_ms: int | None
    deadline_ms: int | None

class BackoffConfig:
    def __init__(self, config: BackoffConfigDict | None = None) -> None: ...

class SocketHandlerBuilder:
    def __init__(self) -> None: ...
    def with_tcp(self, host: str, port: int) -> Self: ...
    def with_unix_path(self, path: str) -> Self: ...
    def with_capacity(self, capacity: int) -> Self: ...
    def with_connect_timeout_ms(self, timeout_ms: int) -> Self: ...
    def with_write_timeout_ms(self, timeout_ms: int) -> Self: ...
    def with_max_frame_size(self, size: int) -> Self: ...
    def with_tls(self, domain: str | None = ..., *, insecure: bool = ...) -> Self: ...
    def with_backoff(self, config: BackoffConfig) -> Self: ...
    def as_dict(self) -> dict[str, object]: ...
    def build(self) -> FemtoSocketHandler: ...

class HTTPHandlerBuilder:
    def __init__(self) -> None: ...
    def with_url(self, url: str) -> Self: ...
    def with_method(self, method: str) -> Self: ...
    def with_basic_auth(self, username: str, password: str) -> Self: ...
    def with_bearer_token(self, token: str) -> Self: ...
    def with_headers(self, headers: Mapping[str, str]) -> Self: ...
    def with_capacity(self, capacity: int) -> Self: ...
    def with_connect_timeout_ms(self, timeout_ms: int) -> Self: ...
    def with_write_timeout_ms(self, timeout_ms: int) -> Self: ...
    def with_backoff(self, config: BackoffConfig) -> Self: ...
    def with_json_format(self) -> Self: ...
    def with_record_fields(self, fields: list[str]) -> Self: ...
    def as_dict(self) -> dict[str, object]: ...
    def build(self) -> FemtoHTTPHandler: ...

ConfigBuilder: _Any
LoggerConfigBuilder: _Any
FormatterBuilder: _Any
LevelFilterBuilder: _Any
NameFilterBuilder: _Any
FilterBuildError: type[Exception]
HandlerConfigError: type[Exception]
HandlerIOError: type[Exception]

hello: _Any
get_logger: _Any
reset_manager_py: _Any
setup_rust_logging: Callable[[], None]

def debug(message: str, /, *, name: str | None = ...) -> str | None:
    """Log a message at DEBUG level via the root logger (or named logger)."""
    ...

def info(message: str, /, *, name: str | None = ...) -> str | None:
    """Log a message at INFO level via the root logger (or named logger)."""
    ...

def warn(message: str, /, *, name: str | None = ...) -> str | None:
    """Log a message at WARN level via the root logger (or named logger)."""
    ...

def error(message: str, /, *, name: str | None = ...) -> str | None:
    """Log a message at ERROR level via the root logger (or named logger)."""
    ...

def _emit_rust_log(level: LevelArg, message: str, target: str | None = ...) -> None: ...
def parse_ini_file(
    path: str, encoding: str | None = ...
) -> list[tuple[str, list[tuple[str, str]]]]: ...

# Frame filtering functions
def filter_frames(
    payload: Mapping[str, _Any],
    *,
    exclude_filenames: list[str] | None = ...,
    exclude_functions: list[str] | None = ...,
    max_depth: int | None = ...,
    exclude_logging: bool = ...,
) -> dict[str, _Any]:
    """Filter frames from a stack_info or exc_info payload.

    Parameters
    ----------
    payload
        The stack_info or exc_info dict from a log record.
    exclude_filenames
        Filename patterns to exclude (substring matching).
    exclude_functions
        Function name patterns to exclude (substring matching).
    max_depth
        Maximum number of frames to retain (keeps most recent).
    exclude_logging
        If True, exclude common logging infrastructure frames
        (femtologging, logging module internals).

    Returns
    -------
    dict
        A new payload dict with frames filtered.

    """
    ...

def get_logging_infrastructure_patterns() -> list[str]:
    """Return the list of filename patterns used by exclude_logging.

    This is useful for inspecting or extending the default patterns.

    Returns
    -------
    list[str]
        The default logging infrastructure filename patterns.

    """
    ...
