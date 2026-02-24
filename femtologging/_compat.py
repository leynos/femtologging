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

from ._femtologging_rs import FemtoLogger, get_logger

_MISSING: object = object()

getLogger = get_logger  # noqa: N816  # TODO(#343): camelCase alias for stdlib compat


def _exception_wrapper(
    self: FemtoLogger,
    message: str,
    /,
    *,
    exc_info: object = _MISSING,
    stack_info: bool = False,
) -> str | None:
    """Log at ERROR level, defaulting ``exc_info`` to ``True`` when omitted.

    Unlike the Rust ``_exception_impl`` (which cannot distinguish omitted
    from explicit ``None``), this wrapper respects an explicit
    ``exc_info=None`` as falsy — matching stdlib ``logging`` semantics.
    """
    if exc_info is _MISSING:
        return self._exception_impl(
            message, exc_info=True, stack_info=stack_info
        )
    return self._exception_impl(
        message, exc_info=typ.cast("typ.Any", exc_info), stack_info=stack_info
    )


FemtoLogger.exception = _exception_wrapper  # type: ignore[assignment,method-assign]

__all__ = ["getLogger"]
