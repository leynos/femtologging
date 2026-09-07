"""Timed rotating file handler argument parsing and validation.

This module extracts all positional/keyword argument processing logic for
TimedRotatingFileHandler from dictConfig, reducing cyclomatic complexity
in the main config module.
"""

from __future__ import annotations

import typing as typ

_UNSUPPORTED_STDLIB_PARAMS: typ.Final[dict[str, tuple[object, str]]] = {
    "encoding": (None, "must be None"),
    "delay": (False, "must be False"),
    "errors": (None, "must be None"),
}

_TIMED_ROTATION_POS_ARGS: typ.Final[tuple[str, ...]] = (
    "when",
    "interval",
    "backup_count",
    "encoding",
    "delay",
    "utc",
    "at_time",
    "errors",
)

_STDLIB_ONLY_SLOTS: typ.Final[frozenset[str]] = frozenset({
    "encoding",
    "delay",
    "errors",
})

_ALIAS_MAP: typ.Final[dict[str, str]] = {
    "filename": "path",
    "backupCount": "backup_count",
    "atTime": "at_time",
}


def _validate_stdlib_unsupported_param(name: str, value: object) -> None:
    """Validate that unsupported stdlib parameters have default values."""
    entry = _UNSUPPORTED_STDLIB_PARAMS.get(name)
    if entry is not None and value is not entry[0]:
        msg = f"{name} parameter is not supported ({entry[1]})"
        raise ValueError(msg)


def _check_alias_conflict(
    kwargs_d: dict[str, object],
    alias_map: dict[str, str],
) -> None:
    """Check for conflicts between aliased and canonical parameter names."""
    conflict = next(
        (
            (canon, alias)
            for alias, canon in alias_map.items()
            if alias in kwargs_d and canon in kwargs_d
        ),
        None,
    )
    if conflict:
        canon, alias = conflict
        msg = f"cannot specify both '{canon}' and '{alias}'"
        raise ValueError(msg)


def _remap_timed_handler_kwargs(kwargs_d: dict[str, object]) -> None:
    """Remap stdlib-style keyword arguments to femtologging conventions."""
    _check_alias_conflict(kwargs_d, _ALIAS_MAP)

    # Remap aliases
    for alias, canon in _ALIAS_MAP.items():
        if canon not in kwargs_d and alias in kwargs_d:
            kwargs_d[canon] = kwargs_d.pop(alias)


def _validate_and_strip_stdlib_only_kwargs(kwargs_d: dict[str, object]) -> None:
    """Validate stdlib-only keyword defaults, then remove them before binding."""
    for name in _STDLIB_ONLY_SLOTS:
        if name in kwargs_d:
            value = kwargs_d.pop(name)
            _validate_stdlib_unsupported_param(name, value)


def _unpack_positional(
    args_t: tuple[object, ...],
    kwargs_d: dict[str, object],
) -> str:
    """Map positional args for a timed rotating handler into ``kwargs_d``.

    Validates stdlib-compatible positional args and rejects unsupported
    features.

    Returns
    -------
    str
        The path extracted from the first positional argument.

    Raises
    ------
    TypeError
        If ``path`` is missing, is not a ``str``, too many positional
        arguments are supplied, or an argument is given both positionally and
        as a keyword.

    Notes
    -----
    :func:`_validate_stdlib_unsupported_param` additionally raises
    ``ValueError`` when a stdlib-only parameter is given a non-default value.

    """
    # Guard: missing path
    if not args_t:
        msg = "expected at least one positional argument 'path'"
        raise TypeError(msg)

    # Guard: non-str path
    if not isinstance(args_t[0], str):
        msg = (
            f"expected first positional argument 'path' to be str, "
            f"got {type(args_t[0]).__name__}"
        )
        raise TypeError(msg)

    path = args_t[0]

    # Guard: too many args
    if len(args_t) > len(_TIMED_ROTATION_POS_ARGS) + 1:  # +1 for path
        max_args = len(_TIMED_ROTATION_POS_ARGS) + 1
        msg = (
            f"too many positional arguments: "
            f"expected at most {max_args}, got {len(args_t)}"
        )
        raise TypeError(msg)

    # Process remaining positional args
    for name, value in zip(_TIMED_ROTATION_POS_ARGS, args_t[1:], strict=False):
        # Check for duplicate
        if name in kwargs_d:
            msg = (
                f"duplicate argument: '{name}' provided both positionally "
                f"and as keyword"
            )
            raise TypeError(msg)

        # Validate stdlib param
        _validate_stdlib_unsupported_param(name, value)

        # Skip stdlib-only slots - validate but don't forward
        if name not in _STDLIB_ONLY_SLOTS:
            kwargs_d[name] = value

    return path


def parse_timed_args(
    args_t: tuple[object, ...],
    kwargs_d: dict[str, object],
) -> tuple[str, object | None]:
    """Parse TimedRotatingFileHandler arguments into (path, options).

    Handles both positional and keyword argument styles, validates stdlib
    parameter compatibility, and constructs TimedHandlerOptions if needed.

    Parameters
    ----------
    args_t : tuple[object, ...]
        Positional arguments; the first is the path when present.
    kwargs_d : dict[str, object]
        Keyword arguments, modified in place during parsing.

    Returns
    -------
    tuple[str, object | None]
        The path and either a ``TimedHandlerOptions`` instance or ``None``
        when no options were supplied.

    Raises
    ------
    TypeError
        If an argument is supplied both positionally and as a keyword.

    Notes
    -----
    The helpers invoked here also raise ``ValueError`` for unsupported stdlib
    parameters or aliasing conflicts, ``TypeError`` for invalid argument
    types, and ``HandlerConfigError`` when the required ``path`` argument is
    missing or is not a string.

    """
    from . import _femtologging_rs as rust

    handler_config_error = getattr(rust, "HandlerConfigError", Exception)

    _remap_timed_handler_kwargs(kwargs_d)

    if args_t and "path" in kwargs_d:
        msg = "duplicate argument: 'path' provided both positionally and as keyword"
        raise TypeError(msg)

    if args_t:
        path = _unpack_positional(args_t, kwargs_d)
    else:
        # Enforce presence of 'path'
        if "path" not in kwargs_d:
            msg = "missing required 'path' argument for timed rotating handler"
            raise handler_config_error(msg)
        # Validate path type
        if not isinstance(kwargs_d.get("path"), str):
            msg = "'path' argument must be a string"
            raise handler_config_error(msg)
        path = typ.cast("str", kwargs_d.pop("path"))

    _validate_and_strip_stdlib_only_kwargs(kwargs_d)

    timed_handler_options = getattr(rust, "TimedHandlerOptions", None)
    if timed_handler_options is None or not kwargs_d:
        return path, None
    # kwargs_d is a runtime dict from external config, so its keys cannot be
    # matched against the constructor signature; widen to Any to forward them.
    options = timed_handler_options(**typ.cast("typ.Any", kwargs_d))
    return path, options


__all__ = ["parse_timed_args"]
