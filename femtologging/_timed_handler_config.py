"""Timed rotating file handler argument parsing and validation.

This module extracts all positional/keyword argument processing logic for
TimedRotatingFileHandler from dictConfig, reducing cyclomatic complexity
in the main config module.
"""

from __future__ import annotations

import typing as typ

cast = typ.cast

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


def _validate_path_arg(args_t: tuple[object, ...]) -> str:
    """Extract and validate the path argument from positional args."""
    if not args_t:
        msg = "expected at least one positional argument 'path'"
        raise TypeError(msg)
    if not isinstance(args_t[0], str):
        msg = (
            f"expected first positional argument 'path' to be str, "
            f"got {type(args_t[0]).__name__}"
        )
        raise TypeError(msg)
    return args_t[0]


def _assign_positional_arg(
    name: str, value: object, kwargs_d: dict[str, object]
) -> None:
    """Assign a positional argument to kwargs_d after validation."""
    if name in kwargs_d:
        msg = f"duplicate argument: '{name}' provided both positionally and as keyword"
        raise TypeError(msg)

    _validate_stdlib_unsupported_param(name, value)

    # Skip stdlib-only slots - validate but don't forward
    if name not in _STDLIB_ONLY_SLOTS:
        kwargs_d[name] = value


def _unpack_positional(
    args_t: tuple[object, ...],
    kwargs_d: dict[str, object],
) -> str:
    """Map positional args for a timed rotating handler into kwargs_d.

    Returns the path extracted from the first positional argument.
    Validates stdlib-compatible positional args and rejects unsupported features.
    """
    path = _validate_path_arg(args_t)

    if len(args_t) > len(_TIMED_ROTATION_POS_ARGS) + 1:  # +1 for path
        max_args = len(_TIMED_ROTATION_POS_ARGS) + 1
        msg = (
            f"too many positional arguments: "
            f"expected at most {max_args}, got {len(args_t)}"
        )
        raise TypeError(msg)

    for name, value in zip(_TIMED_ROTATION_POS_ARGS, args_t[1:], strict=False):
        _assign_positional_arg(name, value, kwargs_d)

    return path


def _remap_timed_handler_kwargs(kwargs_d: dict[str, object]) -> None:
    """Remap stdlib-style keyword arguments to femtologging conventions."""
    # Check for alias conflicts
    conflicts = [
        (canon, alias)
        for alias, canon in _ALIAS_MAP.items()
        if alias in kwargs_d and canon in kwargs_d
    ]
    if conflicts:
        canon, alias = conflicts[0]
        msg = f"cannot specify both '{canon}' and '{alias}'"
        raise ValueError(msg)

    # Remap aliases
    for alias, canon in _ALIAS_MAP.items():
        if canon not in kwargs_d and alias in kwargs_d:
            kwargs_d[canon] = kwargs_d.pop(alias)


def _extract_path_from_kwargs(
    kwargs_d: dict[str, object], handler_config_error: type[Exception]
) -> str:
    """Extract path from kwargs, raising if missing or invalid type."""
    if "path" not in kwargs_d:
        msg = "missing required 'path' argument for timed rotating handler"
        raise handler_config_error(msg)
    val = kwargs_d["path"]
    if not isinstance(val, str):
        msg = f"invalid type for 'path': expected str, got {type(val).__name__}"
        raise handler_config_error(msg)
    return cast(str, kwargs_d.pop("path"))


def _strip_validate_stdlib_only_kwargs(kwargs_d: dict[str, object]) -> None:
    """Validate and remove stdlib-only parameters from kwargs."""
    for param_name in _STDLIB_ONLY_SLOTS:
        if param_name in kwargs_d:
            _validate_stdlib_unsupported_param(param_name, kwargs_d.pop(param_name))


def parse_timed_args(
    args_t: tuple[object, ...],
    kwargs_d: dict[str, object],
) -> tuple[str, object | None]:
    """Parse TimedRotatingFileHandler arguments into (path, options).

    Handles both positional and keyword argument styles, validates stdlib
    parameter compatibility, and constructs TimedHandlerOptions if needed.

    Args:
        args_t: Positional arguments (first arg should be path if present)
        kwargs_d: Keyword arguments (modified in place during parsing)

    Returns:
        Tuple of (path, TimedHandlerOptions or None)

    Raises:
        TypeError: Invalid argument types or duplicates
        ValueError: Unsupported stdlib parameters or conflicts
        HandlerConfigError: Missing required path argument

    """
    from . import _femtologging_rs as rust

    handler_config_error = getattr(rust, "HandlerConfigError", Exception)

    _remap_timed_handler_kwargs(kwargs_d)

    if args_t and "path" in kwargs_d:
        msg = "duplicate argument: 'path' provided both positionally and as keyword"
        raise TypeError(msg)

    path = (
        _unpack_positional(args_t, kwargs_d)
        if args_t
        else _extract_path_from_kwargs(kwargs_d, handler_config_error)
    )

    if not args_t:
        _strip_validate_stdlib_only_kwargs(kwargs_d)

    timed_handler_options = getattr(rust, "TimedHandlerOptions", None)
    if timed_handler_options is None or not kwargs_d:
        return path, None
    # kwargs_d is runtime dict from external config; typ.Any cast needed for **kwargs
    options = timed_handler_options(**cast("typ.Any", kwargs_d))  # pyright: ignore[reportCallIssue]
    return path, options


__all__ = ["parse_timed_args"]
