"""Stdlib-compatible aliases and wrappers for drop-in logging API parity.

Imports come from ``_femtologging_rs`` rather than the parent package to
avoid a circular import (``__init__`` imports from this module).  If a
Python-level wrapper is ever added around ``get_logger``, update the
import source here to keep the alias in lockstep.
"""

from __future__ import annotations

import typing as typ

from ._femtologging_rs import FemtoLogger, get_logger

if typ.TYPE_CHECKING:
    from ._femtologging_rs import ExcInfo

getLogger = get_logger  # noqa: N816 — stdlib-compatible camelCase alias for logging.getLogger


def _exception_wrapper(
    self: FemtoLogger,
    message: str,
    /,
    *,
    exc_info: ExcInfo = True,
    stack_info: bool = False,
) -> str | None:
    """Log at ERROR level with ``exc_info`` defaulting to ``True``.

    Wraps the Rust ``_exception_impl`` so that an omitted ``exc_info``
    receives the Python-side default of ``True`` while an explicit
    ``exc_info=None`` is forwarded faithfully (suppressing capture),
    matching ``logging.Logger.exception()`` semantics.
    """
    return self._exception_impl(message, exc_info=exc_info, stack_info=stack_info)


FemtoLogger.exception = _exception_wrapper  # type: ignore[assignment]

__all__ = ["getLogger"]
