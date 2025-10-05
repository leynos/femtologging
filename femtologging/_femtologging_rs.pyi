from typing import Any as _Any, Final, Literal, Self, Union, overload

FemtoLevel: _Any
LevelName = Literal["TRACE", "DEBUG", "INFO", "WARN", "WARNING", "ERROR", "CRITICAL"]
LevelArg = Union[LevelName, FemtoLevel]

FemtoLogger: _Any
FemtoHandler: _Any
FemtoStreamHandler: _Any
FemtoFileHandler: _Any

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
    def with_formatter(self, fmt: str) -> Self: ...
    def with_max_bytes(self, max_bytes: int) -> Self: ...
    def with_backup_count(self, count: int) -> Self: ...
    @overload
    def with_overflow_policy(
        self, policy: Literal["timeout"], timeout_ms: int
    ) -> Self: ...
    @overload
    def with_overflow_policy(self, policy: Literal["drop", "block"]) -> Self: ...
    def with_overflow_policy(
        self,
        policy: Literal["drop", "block", "timeout"],
        timeout_ms: int | None = ...,
    ) -> Self: ...
    def as_dict(self) -> dict[str, object]: ...
    def build(self) -> FemtoRotatingFileHandler: ...

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
