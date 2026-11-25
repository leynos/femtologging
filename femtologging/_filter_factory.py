"""Resolve stdlib-style ``dictConfig`` filter factory paths.

:func:`resolve_factory` is used by :func:`femtologging._config_filters.
build_filter_from_dict` when a ``"()"`` entry names a dotted string. Resolution
imports modules and reads attributes, so importing the target module already
runs its import-time code before any factory call. That makes the path
trusted-code-equivalent and is why lower-trust callers reject ``"()"`` entries
outright. See ``docs/users-guide.md#filter-factory-trust-boundary``.
"""

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

    Notes
    -----
    Resolution imports modules and reads attributes before the resolved object
    is invoked. Importing a module runs its top-level code, so the path is
    trusted-code-equivalent even before any callable is called. This helper
    provides no sandbox: it cannot make an untrusted path safe. Authorize the
    exact target and trust the code that resolves it before resolution, never
    after it, and prefer rejecting ``"()"`` entries outright for lower-trust
    configuration. Factories invoked
    by :func:`femtologging.dictConfig` run with the remaining entry keys as
    keyword arguments; see ``docs/users-guide.md#filter-factory-trust-boundary``.

    """
    if not dotted_path or "." not in dotted_path:
        msg = f"invalid filter factory path {dotted_path!r}"
        raise ValueError(msg)

    parts = dotted_path.split(".")
    return _resolve_from_root(parts, dotted_path)
