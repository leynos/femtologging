import collections.abc as cabc
import typing as typ

Callable = cabc.Callable
Mapping = cabc.Mapping
_Any = typ.Any
Final = typ.Final
Literal = typ.Literal
Self = typ.Self
Union = typ.Union

FemtoLevel: _Any
LevelName = Literal["TRACE", "DEBUG", "INFO", "WARN", "WARNING", "ERROR", "CRITICAL"]
LevelArg = Union[LevelName, FemtoLevel]

FemtoLogger: _Any
FemtoHandler: _Any
FemtoStreamHandler: _Any
FemtoFileHandler: _Any
FemtoSocketHandler: _Any

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
    policy: Literal["drop", "block", "timeout"]
    max_bytes: int
    backup_count: int

    def __init__(
        self,
        capacity: int = ...,
        flush_interval: int = ...,
        policy: Literal["drop", "block", "timeout"] = ...,
        rotation: tuple[int, int] | None = ...,
    ) -> None: ...

ROTATION_VALIDATION_MSG: Final[str]
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
    def flush(self) -> bool: ...
    def close(self) -> None: ...

class RotatingFileHandlerBuilder:
    def __init__(self, path: str) -> None: ...
    def with_capacity(self, capacity: int) -> Self: ...
    def with_flush_record_interval(self, interval: int) -> Self: ...
    def with_formatter(
        self, fmt: str | Callable[[Mapping[str, object]], str]
    ) -> Self: ...
    def with_max_bytes(self, max_bytes: int) -> Self: ...
    def with_backup_count(self, count: int) -> Self: ...
    def with_overflow_policy(self, policy: OverflowPolicy) -> Self: ...
    def as_dict(self) -> dict[str, object]: ...
    def build(self) -> FemtoRotatingFileHandler: ...

class SocketHandlerBuilder:
    def __init__(self) -> None: ...
    def with_tcp(self, host: str, port: int) -> Self: ...
    def with_unix_path(self, path: str) -> Self: ...
    def with_capacity(self, capacity: int) -> Self: ...
    def with_connect_timeout_ms(self, timeout_ms: int) -> Self: ...
    def with_write_timeout_ms(self, timeout_ms: int) -> Self: ...
    def with_max_frame_size(self, size: int) -> Self: ...
    def with_tls(self, domain: str | None = ..., *, insecure: bool = ...) -> Self: ...
    def with_backoff(
        self,
        base_ms: int | None = ...,
        cap_ms: int | None = ...,
        reset_after_ms: int | None = ...,
        deadline_ms: int | None = ...,
    ) -> Self: ...
    def as_dict(self) -> dict[str, object]: ...
    def build(self) -> FemtoSocketHandler: ...

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

def parse_ini_file(
    path: str, encoding: str | None = ...
) -> list[tuple[str, list[tuple[str, str]]]]: ...
