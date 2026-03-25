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

_STDLIB_ONLY_SLOTS: typ.Final[frozenset[str]] = frozenset(
    {"encoding", "delay", "errors"}
)


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
        msg = (
            f"duplicate argument: '{name}' provided both "
            f"positionally and as keyword"
        )
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

    for i, value in enumerate(args_t[1:]):
        _assign_positional_arg(_TIMED_ROTATION_POS_ARGS[i], value, kwargs_d)

    return path


def _remap_timed_handler_kwargs(kwargs_d: dict[str, object]) -> None:
    """Remap stdlib-style keyword arguments to femtologging conventions."""
    # Check for alias conflicts
    if "path" in kwargs_d and "filename" in kwargs_d:
        msg = "cannot specify both 'path' and 'filename'"
        raise ValueError(msg)
    if "backup_count" in kwargs_d and "backupCount" in kwargs_d:
        msg = "cannot specify both 'backup_count' and 'backupCount'"
        raise ValueError(msg)
    if "at_time" in kwargs_d and "atTime" in kwargs_d:
        msg = "cannot specify both 'at_time' and 'atTime'"
        raise ValueError(msg)

    # Remap aliases
    if "path" not in kwargs_d and "filename" in kwargs_d:
        kwargs_d["path"] = kwargs_d.pop("filename")
    if "backupCount" in kwargs_d and "backup_count" not in kwargs_d:
        kwargs_d["backup_count"] = kwargs_d.pop("backupCount")
    if "atTime" in kwargs_d and "at_time" not in kwargs_d:
        kwargs_d["at_time"] = kwargs_d.pop("atTime")


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

    # Extract path from positional or keyword arguments
    if args_t:
        if "path" in kwargs_d:
            msg = (
                "duplicate argument: 'path' provided both positionally and as keyword"
            )
            raise TypeError(msg)
        path = _unpack_positional(args_t, kwargs_d)
    else:
        if "path" not in kwargs_d:
            msg = "missing required 'path' argument for timed rotating handler"
            raise handler_config_error(msg)
        path = cast(str, kwargs_d.pop("path"))
        # Validate and strip stdlib-only params from kwargs path
        for param_name in ("encoding", "delay", "errors"):
            if param_name in kwargs_d:
                _validate_stdlib_unsupported_param(
                    param_name, kwargs_d.pop(param_name)
                )

    # Construct TimedHandlerOptions if kwargs remain
    timed_handler_options = getattr(rust, "TimedHandlerOptions", None)
    if timed_handler_options is None:
        return path, None

    options = (
        timed_handler_options(**cast("typ.Any", kwargs_d)) if kwargs_d else None
    )
    return path, options


__all__ = ["parse_timed_args"]
