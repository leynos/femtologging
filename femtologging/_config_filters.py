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


def validate_filter_config_keys(fid: str, data: dict[str, object]) -> None:
    """Ensure ``data`` is either declarative or factory-based."""
    present = {"level", "name"} & set(data.keys())
    has_factory = "()" in data
    if has_factory and present:
        msg = f"filter {fid!r} must not mix '()' with 'level' or 'name'"
        raise ValueError(msg)
    if has_factory:
        return
    if not present:
        msg = f"filter {fid!r} must contain a 'level', 'name', or '()' key"
        raise ValueError(msg)
    if len(present) > 1:
        msg = f"filter {fid!r} must contain 'level' or 'name', not both"
        raise ValueError(msg)
    unknown = set(data.keys()) - {"level", "name"}
    if unknown:
        msg = f"filter {fid!r} has unsupported keys: {sorted(unknown)!r}"
        raise ValueError(msg)


def build_filter_from_dict(fid: str, data: dict[str, object]) -> object:
    """Create a filter builder from ``dictConfig`` filter data."""
    validate_filter_config_keys(fid, data)
    if "()" in data:
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
