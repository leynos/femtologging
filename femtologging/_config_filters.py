"""Filter helpers for ``dictConfig`` parsing."""

from __future__ import annotations

import typing as typ

from ._config_policy import factory_disallowed_message
from ._femtologging_rs import (
    LevelFilterBuilder,
    NameFilterBuilder,
    PythonCallbackFilterBuilder,
)
from ._filter_factory import resolve_factory

_DECLARATIVE_KEYS: frozenset[str] = frozenset({"level", "name"})


# Validation helpers
def _validate_factory_keys(fid: str, present: set[str]) -> None:
    """Reject factory filters mixed with declarative keys."""
    if present:
        msg = f"filter {fid!r} must not mix '()' with 'level' or 'name'"
        raise ValueError(msg)


def _validate_declarative_keys(
    fid: str, present: set[str], data: dict[str, object]
) -> None:
    """Reject invalid declarative filter key combinations."""
    if not present:
        msg = f"filter {fid!r} must contain a 'level', 'name', or '()' key"
        raise ValueError(msg)
    if len(present) > 1:
        msg = f"filter {fid!r} must contain 'level' or 'name', not both"
        raise ValueError(msg)
    unknown = set(data.keys()) - _DECLARATIVE_KEYS
    if unknown:
        msg = f"filter {fid!r} has unsupported keys: {sorted(unknown)!r}"
        raise ValueError(msg)


def validate_filter_config_keys(fid: str, data: dict[str, object]) -> None:
    """Ensure ``data`` is either declarative or factory-based."""
    present = set(_DECLARATIVE_KEYS & set(data.keys()))
    if "()" in data:
        _validate_factory_keys(fid, present)
    else:
        _validate_declarative_keys(fid, present, data)


# Builder helpers
def _build_factory_filter(fid: str, data: dict[str, object]) -> object:
    """Build a callback filter from a factory reference."""
    factory_ref = data["()"]
    if isinstance(factory_ref, str):
        factory = resolve_factory(factory_ref)
    else:
        factory = factory_ref
    if not callable(factory):
        msg = f"filter {fid!r} factory must be callable"
        raise TypeError(msg)
    kwargs = {key: value for key, value in data.items() if key != "()"}
    # The factory comes from user configuration, so its signature is unknown;
    # widening to Any is the only way to forward arbitrary keyword arguments.
    built = typ.cast("typ.Any", factory)(**kwargs)
    return PythonCallbackFilterBuilder(built)


def _build_declarative_filter(fid: str, data: dict[str, object]) -> object:
    """Build a declarative level- or name-based filter."""
    if "level" in data:
        level = data["level"]
        if not isinstance(level, str):
            msg = f"filter {fid!r} level must be a string"
            raise TypeError(msg)
        return LevelFilterBuilder().with_max_level(level)
    name = data["name"]
    if not isinstance(name, str):
        msg = f"filter {fid!r} name must be a string"
        raise TypeError(msg)
    return NameFilterBuilder().with_prefix(name)


def build_filter_from_dict(
    fid: str, data: dict[str, object], *, allow_factories: bool = True
) -> object:
    """Create a filter builder from ``dictConfig`` filter data.

    Parameters
    ----------
    fid
        Filter identifier used in error messages and registry lookups.
    data
        Filter entry body. A ``"()"`` key selects the stdlib factory form; an
        entry without it must declare exactly one of ``level`` or ``name``.
    allow_factories
        When ``False``, reject any entry carrying a ``"()"`` key. The rejection
        happens on key presence alone, so the factory target is never resolved
        or invoked. Declarative ``level`` and ``name`` entries still work.

    Returns
    -------
    object
        A ``LevelFilterBuilder``, ``NameFilterBuilder``, or
        ``PythonCallbackFilterBuilder``.

    Raises
    ------
    ValueError
        If the entry is malformed, or uses a factory while
        ``allow_factories`` is ``False``.

    Examples
    --------
    >>> builder = build_filter_from_dict("lvl", {"level": "INFO"})
    >>> build_filter_from_dict("f", {"()": "pkg.Factory"}, allow_factories=False)
    Traceback (most recent call last):
    ...
    ValueError: filter 'f' uses a '()' factory, rejected by allow_factories=False

    """
    # Snapshot key presence once: a mapping that mutates during iteration must
    # not let the guard pass and then reach factory construction.
    has_factory = "()" in data
    if not allow_factories and has_factory:
        msg = factory_disallowed_message(fid)
        raise ValueError(msg)
    validate_filter_config_keys(fid, data)
    if has_factory:
        return _build_factory_filter(fid, data)
    return _build_declarative_filter(fid, data)
