from __future__ import annotations

from enum import Enum


class OverflowPolicy(Enum):
    """Behaviour when a handler queue is full."""

    DROP = "drop"
    BLOCK = "block"
    TIMEOUT = "timeout"

    def __str__(self) -> str:
        return self.value
