"""Helpers for stdlib-style ``dictConfig`` filter factories."""

from __future__ import annotations

import importlib
import typing as typ

Any = typ.Any


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
        module_name = ".".join(parts[:index])
        try:
            resolved: object = importlib.import_module(module_name)
        except ImportError:
            continue

        try:
            for attr in parts[index:]:
                resolved = getattr(resolved, attr)
        except AttributeError as exc:
            msg = f"failed to resolve filter factory {dotted_path!r}"
            raise ValueError(msg) from exc
        return resolved

    msg = f"failed to import filter factory {dotted_path!r}"
    raise ValueError(msg)
