"""Scoped structured logging context helpers."""

from __future__ import annotations

import typing as typ
from contextlib import contextmanager

from ._femtologging_rs import _pop_log_context, _push_log_context

if typ.TYPE_CHECKING:
    import collections.abc as cabc


@contextmanager
def log_context(**fields: object) -> cabc.Iterator[None]:
    """Temporarily attach structured key-values to ``FemtoLogger`` records.

    Parameters
    ----------
    **fields : object
        Arbitrary key-value pairs to attach as structured metadata to records
        emitted through ``FemtoLogger`` methods, including loggers returned by
        ``get_logger()``, on the calling thread. Keys must be strings no
        longer than 64 UTF-8 bytes. Values must be `str`, `int`, `float`,
        `bool`, or `None`. Duplicate keys override outer context values.

    Yields
    ------
    None
        Context manager that pushes the provided fields onto the thread-local
        logging context stack on entry and pops them on exit.

    Raises
    ------
    ValueError
        If the context exceeds a key-count or byte-size limit.
    TypeError
        If a value has an unsupported type (not str/int/float/bool/None).

    Notes
    -----
    Context values are merged on the producer thread before queueing. The
    ``extra`` mapping accepted by ``FemtoLogger.log()``, ``debug()``,
    ``info()``, ``warning()``, ``error()``, and ``critical()`` overrides
    scoped context keys with the same name. Rust
    ``tracing``-bridge events use span fields instead of this scoped context.

    The context stack is thread-local, not task-local. Holding this context
    across an ``await`` in a single-threaded event loop can share fields with
    other in-flight tasks. Async callers should pass per-call fields through
    ``extra`` instead.

    """
    _push_log_context(fields)
    try:
        yield
    finally:
        _pop_log_context()
