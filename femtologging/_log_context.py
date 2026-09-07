"""Scoped structured logging context helpers."""

from __future__ import annotations

import typing as typ
from contextlib import contextmanager

from ._femtologging_rs import _pop_log_context, _push_log_context

if typ.TYPE_CHECKING:
    import collections.abc as cabc


@contextmanager
def log_context(**fields: object) -> cabc.Iterator[None]:
    """Temporarily attach structured key-values to log records on this thread.

    Parameters
    ----------
    **fields : object
        Arbitrary key-value pairs to attach as structured metadata to all log
        records emitted on the current thread while this context is active.
        Keys must be valid Python identifiers. Values must be `str`, `int`,
        `float`, `bool`, or `None`. Duplicate keys override outer context values.

    Yields
    ------
    None
        Context manager that pushes the provided fields onto the thread-local
        logging context stack on entry and pops them on exit.

    Notes
    -----
    Context values are merged on the producer thread before queueing. Inline
    structured fields emitted by Rust macros override outer context keys.

    The Rust ``_push_log_context`` primitive raises ``ValueError`` when a key
    is invalid (empty, too long, or not a valid identifier) and ``TypeError``
    when a value has an unsupported type (not ``str``/``int``/``float``/
    ``bool``/``None``).

    """
    _push_log_context(fields)
    try:
        yield
    finally:
        _pop_log_context()
