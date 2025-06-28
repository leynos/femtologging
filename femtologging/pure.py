from __future__ import annotations


class FemtoLogger:
    """Simplistic Python implementation used when the Rust module is missing."""

    def __init__(self, name: str) -> None:
        self.name = name

    def log(self, level: str, message: str) -> str:
        """Return the formatted log message."""
        return f"{self.name} [{level}] {message}"


def hello() -> str:
    """Return a friendly greeting from Python."""
    return "hello from Python"


class FemtoStreamHandler:
    """Placeholder used when the Rust extension is unavailable."""

    def __init__(self) -> None:  # pragma: no cover - simple stub
        pass


class FemtoHandler:
    """Base class placeholder when the Rust extension is missing."""

    def handle(self, _record: object) -> None:  # pragma: no cover - stub
        pass


__all__ = ["FemtoHandler", "FemtoLogger", "FemtoStreamHandler", "hello"]
