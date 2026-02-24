"""Stdlib-compatible aliases for drop-in logging API parity."""

from __future__ import annotations

from ._femtologging_rs import get_logger as getLogger  # noqa: N812

__all__ = ["getLogger"]
