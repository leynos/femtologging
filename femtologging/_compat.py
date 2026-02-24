"""Stdlib-compatible aliases for drop-in logging API parity.

Purpose
-------
Expose aliases that mirror the standard-library ``logging`` module's
public names so that callers can use ``getLogger`` as a drop-in
replacement for ``logging.getLogger``.

Notes
-----
Imports come from ``_femtologging_rs`` rather than the parent package
to avoid a circular import (``__init__`` imports from this module).
If a Python-level wrapper is ever added around ``get_logger``, the
import source here must be kept in sync so the alias stays correct.

Examples
--------
>>> from femtologging import getLogger
>>> logger = getLogger("myapp.auth")
>>> logger.info("user logged in") is not None
True
"""

from __future__ import annotations

from ._femtologging_rs import get_logger

getLogger = get_logger  # noqa: N816  # FIXME(#343): camelCase alias for stdlib compat

__all__ = ["getLogger"]
