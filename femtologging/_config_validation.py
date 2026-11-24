"""Shared type validation for ``dictConfig`` parsing.

``dictConfig`` sections accept loosely typed mappings, so every section parser
needs the same small checks: is this a mapping rather than bytes, are its keys
strings, is this a list of strings. Keeping them here lets the section modules
depend on one leaf module instead of importing them back out of
:mod:`femtologging.config`.

The helpers are deliberately narrow. They validate shape only; the
section-specific rules live with the section that applies them.
"""

from __future__ import annotations

import collections.abc as cabc
import typing as typ


def _validate_mapping_type(value: object, name: str) -> cabc.Mapping[object, object]:
    """Ensure ``value`` is a mapping and not bytes-like."""
    if isinstance(value, (bytes, bytearray)) or not isinstance(value, cabc.Mapping):
        msg = f"{name} must be a mapping"
        raise TypeError(msg)
    return typ.cast("cabc.Mapping[object, object]", value)


def _validate_string_keys(
    mapping: cabc.Mapping[object, object], name: str
) -> cabc.Mapping[str, object]:
    """Ensure all keys in ``mapping`` are strings."""
    for key in mapping:
        if not isinstance(key, str):
            msg = f"{name} keys must be strings"
            raise TypeError(msg)
    return typ.cast("cabc.Mapping[str, object]", mapping)


def _validate_string_list(
    value: object, label: str, *, owner: str = "logger"
) -> list[str]:
    """Validate that ``value`` is a list or tuple of strings."""
    if not isinstance(value, (list, tuple)):
        msg = f"{owner} {label} must be a list or tuple of strings"
        raise TypeError(msg)
    seq = typ.cast("cabc.Sequence[object]", value)
    if not all(isinstance(item, str) for item in seq):
        msg = f"{owner} {label} must be a list or tuple of strings"
        raise TypeError(msg)
    return list(typ.cast("cabc.Sequence[str]", seq))
