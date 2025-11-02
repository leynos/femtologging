"""Overflow policy factory helpers exposed by the Rust extension."""

from __future__ import annotations

from ._femtologging_rs import OverflowPolicy

__all__ = ["OverflowPolicy"]
