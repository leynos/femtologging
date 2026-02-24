"""femtologging package."""

from __future__ import annotations

from . import _femtologging_rs as rust
from ._basic_config import BasicConfig, basicConfig
from ._compat import (
    getLogger as getLogger,
)
from ._femtologging_rs import (
    EXCEPTION_SCHEMA_VERSION,
    ROTATION_VALIDATION_MSG,
    BackoffConfig,
    ConfigBuilder,
    FemtoFileHandler,
    FemtoHandler,
    FemtoHTTPHandler,
    FemtoLogger,
    FemtoRotatingFileHandler,
    FemtoSocketHandler,
    FemtoStreamHandler,
    FileHandlerBuilder,
    FilterBuildError,
    FormatterBuilder,
    HandlerConfigError,
    HandlerIOError,
    HandlerOptions,
    HTTPHandlerBuilder,
    LevelFilterBuilder,
    LoggerConfigBuilder,
    NameFilterBuilder,
    RotatingFileHandlerBuilder,
    SocketHandlerBuilder,
    StreamHandlerBuilder,
    debug,
    error,
    filter_frames,
    get_logger,
    get_logging_infrastructure_patterns,
    hello,
    info,
    warn,
)
from ._femtologging_rs import (
    reset_manager_py as reset_manager,
)
from ._rust_compat import (
    _clear_rotating_fresh_failure_for_test as _clear_rotating_fresh_failure_for_test,
)
from ._rust_compat import (
    _force_rotating_fresh_failure_for_test as _force_rotating_fresh_failure_for_test,
)
from ._rust_compat import (
    setup_rust_logging,
)
from .adapter import StdlibHandlerAdapter
from .config import dictConfig
from .file_config import fileConfig
from .overflow_policy import OverflowPolicy

__all__ = [
    "EXCEPTION_SCHEMA_VERSION",
    "ROTATION_VALIDATION_MSG",
    "BackoffConfig",
    "BasicConfig",
    "ConfigBuilder",
    "FemtoFileHandler",
    "FemtoHTTPHandler",
    "FemtoHandler",
    "FemtoLogger",
    "FemtoRotatingFileHandler",
    "FemtoSocketHandler",
    "FemtoStreamHandler",
    "FileHandlerBuilder",
    "FilterBuildError",
    "FormatterBuilder",
    "HTTPHandlerBuilder",
    "HandlerConfigError",
    "HandlerIOError",
    "HandlerOptions",
    "LevelFilterBuilder",
    "LoggerConfigBuilder",
    "NameFilterBuilder",
    "OverflowPolicy",
    "RotatingFileHandlerBuilder",
    "SocketHandlerBuilder",
    "StdlibHandlerAdapter",
    "StreamHandlerBuilder",
    "basicConfig",
    "debug",
    "dictConfig",
    "error",
    "fileConfig",
    "filter_frames",
    "getLogger",
    "get_logger",
    "get_logging_infrastructure_patterns",
    "hello",
    "info",
    "reset_manager",
    "rust",
    "setup_rust_logging",
    "warn",
]
