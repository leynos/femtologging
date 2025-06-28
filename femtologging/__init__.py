"""femtologging package."""

from __future__ import annotations

PACKAGE_NAME = "femtologging"

try:  # pragma: no cover - Rust optional
    rust = __import__(f"_{PACKAGE_NAME}_rs")
    hello = rust.hello  # type: ignore[attr-defined]
    FemtoLogger = rust.FemtoLogger  # type: ignore[attr-defined]
    FemtoStreamHandler = rust.FemtoStreamHandler  # type: ignore[attr-defined]
except ModuleNotFoundError:  # pragma: no cover - Python fallback
    from .pure import FemtoLogger, FemtoStreamHandler, hello

__all__ = ["FemtoLogger", "FemtoStreamHandler", "hello"]
