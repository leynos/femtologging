"""Helpers for stdlib-style ``dictConfig`` filter factories."""

from __future__ import annotations

import importlib


def _try_import_module(module_name: str) -> object | None:
    """Import ``module_name`` and return ``None`` on import failure."""
    try:
        return importlib.import_module(module_name)
    except ModuleNotFoundError as exc:
        if exc.name != module_name:
            raise
        return None


def _resolve_from_root(parts: list[str], dotted_path: str) -> object:
    """Resolve a dotted path using stdlib-style attribute-first traversal."""
    module_name = parts[0]
    resolved = _try_import_module(module_name)
    if resolved is None:
        msg = f"failed to import filter factory {dotted_path!r}"
        raise ValueError(msg)

    for attr in parts[1:]:
        try:
            resolved = getattr(resolved, attr)
        except AttributeError:
            module_name = f"{module_name}.{attr}"
            imported = _try_import_module(module_name)
            if imported is None:
                msg = f"failed to resolve filter factory {dotted_path!r}"
                raise ValueError(msg) from None
            resolved = imported

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
    return _resolve_from_root(parts, dotted_path)
