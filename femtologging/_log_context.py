"""Scoped structured logging context helpers."""

from __future__ import annotations

import typing as typ
from contextlib import contextmanager

from ._femtologging_rs import _pop_log_context, _push_log_context


@contextmanager
def log_context(**fields: object) -> typ.Iterator[None]:
    """Temporarily attach structured key-values to log records on this thread."""
    _push_log_context(fields)
    try:
        yield
    finally:
        _pop_log_context()
