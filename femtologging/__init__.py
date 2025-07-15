"""femtologging package."""

from __future__ import annotations

from .overflow_policy import OverflowPolicy

PACKAGE_NAME = "femtologging"

rust = __import__(f"_{PACKAGE_NAME}_rs")

hello = rust.hello  # type: ignore[attr-defined]
FemtoLogger = rust.FemtoLogger  # type: ignore[attr-defined]
get_logger = rust.get_logger  # type: ignore[attr-defined]
reset_manager = rust.reset_manager_py  # type: ignore[attr-defined]
FemtoHandler = rust.FemtoHandler  # type: ignore[attr-defined]
FemtoStreamHandler = rust.FemtoStreamHandler  # type: ignore[attr-defined]
FemtoFileHandler = rust.FemtoFileHandler  # type: ignore[attr-defined]
PyHandlerConfig = rust.PyHandlerConfig  # type: ignore[attr-defined]

__all__ = [
    "FemtoHandler",
    "FemtoLogger",
    "get_logger",
    "reset_manager",
    "FemtoStreamHandler",
    "FemtoFileHandler",
    "PyHandlerConfig",
    "OverflowPolicy",
    "hello",
]
