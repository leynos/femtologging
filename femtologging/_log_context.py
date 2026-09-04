"""Scoped structured logging context helpers."""

from __future__ import annotations

import contextvars
import typing as typ
from contextlib import contextmanager

from ._femtologging_rs import _validate_log_context

_LOG_CONTEXT: contextvars.ContextVar[dict[str, str] | None] = contextvars.ContextVar(
    "femtologging_log_context",
    default=None,
)


def _current_log_context() -> dict[str, str]:
    """Return a copy of the active task-local structured logging context."""
    return (_LOG_CONTEXT.get() or {}).copy()


def _normalise_context_fields(fields: dict[str, object]) -> dict[str, str]:
    """Convert supported context values to their record metadata representation."""
    normalised: dict[str, str] = {}
    for key, value in fields.items():
        if value is None or isinstance(value, str | int | float | bool):
            normalised[key] = str(value)
        else:
            msg = "context values must be str, int, float, bool, or None"
            raise TypeError(msg)
    return normalised


if typ.TYPE_CHECKING:
    import collections.abc as cabc


@contextmanager
def log_context(**fields: object) -> cabc.Iterator[None]:
    """Temporarily attach structured key-values to records in this task.

    Parameters
    ----------
    **fields : object
        Arbitrary key-value pairs to attach as structured metadata to all log
        records emitted in the current Python task while this context is active.
        Keys must be valid Python identifiers. Values must be `str`, `int`,
        `float`, `bool`, or `None`. Duplicate keys override outer context values.

    Yields
    ------
    None
        Context manager that makes the provided fields active for the current
        `contextvars` context and restores the previous value on exit.

    Notes
    -----
    Context values are captured when a Python logging API creates its record
    and are merged before queueing. Rust macros and the Rust `log` and
    `tracing` bridges use Rust scoped context on their emitting OS thread; they
    do not inherit Python task-local fields.

    """
    context = _current_log_context() | _normalise_context_fields(fields)
    _validate_log_context(context)
    token = _LOG_CONTEXT.set(context)
    try:
        yield
    finally:
        _LOG_CONTEXT.reset(token)
