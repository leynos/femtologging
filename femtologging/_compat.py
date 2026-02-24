"""Stdlib-compatible aliases for drop-in logging API parity.

Imports come from ``_femtologging_rs`` rather than the parent package to
avoid a circular import (``__init__`` imports from this module).  If a
Python-level wrapper is ever added around ``get_logger``, update the
import source here to keep the alias in lockstep.
"""

from __future__ import annotations

from ._femtologging_rs import get_logger

getLogger = get_logger  # noqa: N816

__all__ = ["getLogger"]
