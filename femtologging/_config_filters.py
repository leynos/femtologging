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

if typ.TYPE_CHECKING:
    import collections.abc as cabc

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
    fid: str, data: cabc.Mapping[str, object], *, allow_factories: bool = True
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
        When ``False``, reject any entry carrying a ``"()"`` key. The initial
        key scan rejects before any value is read, so a factory target it sees
        is never read, resolved, or invoked. A key that appears only during
        copying is rejected before dispatch, so it is never resolved or
        invoked. Declarative ``level`` and ``name`` entries still work.

    Returns
    -------
    object
        A ``LevelFilterBuilder``, ``NameFilterBuilder``, or
        ``PythonCallbackFilterBuilder``.

    Raises
    ------
    ValueError
        If the entry is malformed, mixes factory and declarative forms, or
        uses a factory while ``allow_factories`` is ``False``.

    Notes
    -----
    When ``allow_factories`` is ``True`` (the default), a ``"()"`` entry uses
    the stdlib factory form: a dotted string is imported and resolved, and the
    resolved callable is invoked with the entry's remaining keys as keyword
    arguments. A direct callable object is invoked without resolution. Both
    forms run code chosen by the configuration author, so the entry is
    trusted-code-equivalent; see
    ``docs/users-guide.md#filter-factory-trust-boundary``. A non-string
    ``level`` or ``name``, or a factory target that is not callable, raises
    ``TypeError`` from the builder helpers.

    Examples
    --------
    >>> from femtologging._config_filters import build_filter_from_dict
    >>> type(build_filter_from_dict("lvl", {"level": "INFO"})).__name__
    'LevelFilterBuilder'
    >>> build_filter_from_dict("f", {"()": "pkg.Factory"}, allow_factories=False)
    Traceback (most recent call last):
    ...
    ValueError: filter 'f' uses a '()' factory, rejected by allow_factories=False

    """
    # Scan keys first: copying a mapping reads every value, so a check after
    # the copy would run a hostile value accessor before the policy rejection.
    if not allow_factories and "()" in list(data):
        msg = factory_disallowed_message(fid)
        raise ValueError(msg)
    # Copy once so the policy re-check, validation, and dispatch all inspect the
    # same keys, even if a dict subclass mutates its mapping during validation.
    entry = dict(data)
    has_factory = "()" in entry
    if not allow_factories and has_factory:
        msg = factory_disallowed_message(fid)
        raise ValueError(msg)
    validate_filter_config_keys(fid, entry)
    if has_factory:
        return _build_factory_filter(fid, entry)
    return _build_declarative_filter(fid, entry)
