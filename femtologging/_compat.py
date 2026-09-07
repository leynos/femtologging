"""Stdlib-compatible wrappers and aliases for drop-in logging API parity.

Purpose
-------
Expose aliases and thin wrappers that mirror the standard-library
``logging`` module's public names so that callers can use ``getLogger``
and ``exception()`` as drop-in replacements.

Notes
-----
Imports come from ``_femtologging_rs`` rather than the parent package
to avoid a circular import (``__init__`` imports from this module).

The ``exception()`` wrapper uses a sentinel to distinguish an omitted
``exc_info`` from an explicit ``None``, working around a PyO3 limitation
where both map to Rust ``Option::None``.

Examples
--------
>>> from femtologging import getLogger
>>> logger = getLogger("myapp.auth")
>>> logger.info("user logged in") is not None
True

"""

from __future__ import annotations

import typing as typ

from ._femtologging_rs import FemtoLogger

# ruff: ignore[lowercase-imported-as-non-lowercase] alias mirrors stdlib
# logging.getLogger, whose camelCase spelling is part of the drop-in API.
from ._femtologging_rs import get_logger as getLogger

if typ.TYPE_CHECKING:
    from ._femtologging_rs import ExcInfo

_MISSING = typ.cast("ExcInfo", object())


def _exception_wrapper(
    self: FemtoLogger,
    message: str,
    /,
    *,
    exc_info: ExcInfo = _MISSING,
    stack_info: bool = False,
) -> str | None:
    """Log at ERROR level, defaulting ``exc_info`` to ``True`` when omitted.

    Unlike the Rust ``_exception_impl`` (which cannot distinguish omitted
    from explicit ``None``), this wrapper respects an explicit
    ``exc_info=None`` as falsy — matching stdlib ``logging`` semantics.

    Returns
    -------
    str or None
        The formatted record emitted by ``_exception_impl``, or ``None``
        when the record was dropped.

    """
    if exc_info is _MISSING:
        return self._exception_impl(message, exc_info=True, stack_info=stack_info)
    # Normalize falsy exc_info (None, False, 0, …) to Python False so that
    # PyO3 receives a concrete PyBool rather than Python None (which PyO3
    # maps to Rust Option::None and would trigger the auto-capture default).
    resolved = exc_info or False
    return self._exception_impl(message, exc_info=resolved, stack_info=stack_info)


_logger_type = typ.cast("type[typ.Any]", FemtoLogger)
# PyO3 classes expose no way to bind a Python-level method statically, so the
# wrapper is patched on at import time via the ``type[typ.Any]`` view above.
_logger_type.exception = typ.cast("typ.Any", _exception_wrapper)

__all__ = ["getLogger"]
