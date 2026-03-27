"""Filter helpers for ``dictConfig`` parsing."""

from __future__ import annotations

import typing as typ

from . import _femtologging_rs as rust
from ._filter_factory import resolve_factory

Any = typ.Any
cast = typ.cast

LevelFilterBuilder = rust.LevelFilterBuilder
NameFilterBuilder = rust.NameFilterBuilder
PythonCallbackFilterBuilder = rust.PythonCallbackFilterBuilder

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
    built = cast("Any", factory)(**kwargs)  # pyright: ignore[reportCallIssue]
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


def build_filter_from_dict(fid: str, data: dict[str, object]) -> object:
    """Create a filter builder from ``dictConfig`` filter data."""
    validate_filter_config_keys(fid, data)
    if "()" in data:
        return _build_factory_filter(fid, data)
    return _build_declarative_filter(fid, data)
