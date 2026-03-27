"""Helpers for stdlib-style ``dictConfig`` filter factories."""

from __future__ import annotations

import importlib
import typing as typ

Any = typ.Any


def _try_import_module(module_name: str) -> object | None:
    """Import ``module_name`` and return ``None`` on import failure."""
    try:
        return importlib.import_module(module_name)
    except ImportError:
        return None


def _resolve_attrs(base: object, attrs: list[str], dotted_path: str) -> object:
    """Resolve ``attrs`` from ``base`` or raise the existing ValueError."""
    resolved = base
    try:
        for attr in attrs:
            resolved = getattr(resolved, attr)
    except AttributeError as exc:
        msg = f"failed to resolve filter factory {dotted_path!r}"
        raise ValueError(msg) from exc
    return resolved


def resolve_factory(dotted_path: str) -> object:
    """Resolve ``dotted_path`` to a Python object.

    Parameters
    ----------
    dotted_path
        Fully qualified import path for a class, function, or other factory.

    Returns
    -------
    object
        The resolved Python object.

    Raises
    ------
    ValueError
        If the path cannot be imported or an attribute lookup fails.

    """
    if not dotted_path or "." not in dotted_path:
        msg = f"invalid filter factory path {dotted_path!r}"
        raise ValueError(msg)

    parts = dotted_path.split(".")
    for index in range(len(parts), 0, -1):
        resolved = _try_import_module(".".join(parts[:index]))
        if resolved is None:
            continue
        return _resolve_attrs(resolved, list(parts[index:]), dotted_path)

    msg = f"failed to import filter factory {dotted_path!r}"
    raise ValueError(msg)
