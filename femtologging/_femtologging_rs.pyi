from typing import Any as _Any, Literal, Union

FemtoLevel: _Any
LevelName = Literal["TRACE", "DEBUG", "INFO", "WARN", "WARNING", "ERROR", "CRITICAL"]
LevelArg = Union[LevelName, FemtoLevel]

FemtoLogger: _Any
FemtoHandler: _Any
FemtoStreamHandler: _Any
FemtoFileHandler: _Any
FemtoRotatingFileHandler: _Any
StreamHandlerBuilder: _Any
FileHandlerBuilder: _Any
RotatingFileHandlerBuilder: _Any
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
