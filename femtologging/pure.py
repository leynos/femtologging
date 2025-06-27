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


__all__ = ["FemtoLogger", "hello"]
