"""Configuration via logging-style dictionaries.

This module implements :func:`dictConfig`, a restricted variant of
``logging.config.dictConfig``. Only a subset of the standard schema is
recognized: ``filters`` sections, handler ``level`` attributes, and
incremental configuration are unsupported.

String level parameters accept case-insensitive names: "TRACE", "DEBUG",
"INFO", "WARN", "WARNING", "ERROR", and "CRITICAL". "WARN" and "WARNING"
are equivalent.

Example:
-------
>>> dictConfig({
...     "version": 1,
...     "handlers": {"h": {"class": "femtologging.StreamHandler"}},
...     "root": {"level": "INFO", "handlers": ["h"]},
... })

The ``dictConfig`` format does not support ``filters`` and will raise
``ValueError`` if a ``filters`` section is provided. To attach filters, use the
builder API:

    cb = (
        ConfigBuilder()
        .with_filter("lvl", LevelFilterBuilder().with_max_level("INFO"))
        .with_logger("core", LoggerConfigBuilder().with_filters(["lvl"]))
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
    )
    cb.build_and_init()

"""

from __future__ import annotations

import ast
import collections.abc as cabc
import typing as typ

from . import _femtologging_rs as rust
from .overflow_policy import OverflowPolicy

Callable = cabc.Callable
Mapping = cabc.Mapping
Sequence = cabc.Sequence
Any = typ.Any
Final = typ.Final
cast = typ.cast

rust = typ.cast("Any", rust)
HandlerConfigError: type[Exception] = getattr(rust, "HandlerConfigError", Exception)
HandlerIOError: type[Exception] = getattr(rust, "HandlerIOError", Exception)

StreamHandlerBuilder = rust.StreamHandlerBuilder
SocketHandlerBuilder = rust.SocketHandlerBuilder
BackoffConfig = getattr(rust, "BackoffConfig", None)
FileHandlerBuilder = rust.FileHandlerBuilder
RotatingFileHandlerBuilder = rust.RotatingFileHandlerBuilder
ConfigBuilder = rust.ConfigBuilder
LoggerConfigBuilder = rust.LoggerConfigBuilder
FormatterBuilder = rust.FormatterBuilder
LevelFilterBuilder = rust.LevelFilterBuilder
NameFilterBuilder = rust.NameFilterBuilder


_HANDLER_CLASS_MAP: typ.Final[dict[str, object]] = {
    "logging.StreamHandler": StreamHandlerBuilder,
    "femtologging.StreamHandler": StreamHandlerBuilder,
    "logging.handlers.SocketHandler": SocketHandlerBuilder,
    "femtologging.SocketHandler": SocketHandlerBuilder,
    "femtologging.FemtoSocketHandler": SocketHandlerBuilder,
    "logging.FileHandler": FileHandlerBuilder,
    "femtologging.FileHandler": FileHandlerBuilder,
    "logging.handlers.RotatingFileHandler": RotatingFileHandlerBuilder,
    "logging.RotatingFileHandler": RotatingFileHandlerBuilder,
    "femtologging.RotatingFileHandler": RotatingFileHandlerBuilder,
    "femtologging.FemtoRotatingFileHandler": RotatingFileHandlerBuilder,
}


def _evaluate_string_safely(value: str, context: str) -> object:
    """Safely evaluate a string ``value`` using ``ast.literal_eval``."""
    try:
        return ast.literal_eval(value)
    except (ValueError, SyntaxError) as exc:
        msg = f"invalid {context}: {value}"
        raise ValueError(msg) from exc


def _validate_mapping_type(value: object, name: str) -> Mapping[object, object]:
    """Ensure ``value`` is a mapping and not bytes-like."""
    if isinstance(value, (bytes, bytearray)) or not isinstance(value, Mapping):
        msg = f"{name} must be a mapping"
        raise TypeError(msg)
    return cast("Mapping[object, object]", value)


def _validate_no_bytes(value: object, name: str) -> None:
    """Reject ``bytes`` or ``bytearray`` for ``value``."""
    if isinstance(value, (bytes, bytearray)):
        msg = f"{name} must not be bytes or bytearray"
        raise TypeError(msg)


def _validate_string_keys(
    mapping: Mapping[object, object], name: str
) -> Mapping[str, object]:
    """Ensure all keys in ``mapping`` are strings."""
    for key in mapping:
        if not isinstance(key, str):
            msg = f"{name} keys must be strings"
            raise TypeError(msg)
    return cast("Mapping[str, object]", mapping)


def _coerce_args(args: object, ctx: str) -> list[object]:
    """Convert ``args`` into a list for handler construction."""
    if isinstance(args, str):
        args = _evaluate_string_safely(args, f"{ctx} args")
    if args is None:
        return []
    _validate_no_bytes(args, f"{ctx} args")
    if not isinstance(args, Sequence):
        msg = f"{ctx} args must be a sequence"
        raise TypeError(msg)
    return list(args)


def _coerce_kwargs(kwargs: object, ctx: str) -> dict[str, object]:
    """Convert ``kwargs`` into a dictionary for handler construction."""
    if isinstance(kwargs, str):
        kwargs = _evaluate_string_safely(kwargs, f"{ctx} kwargs")
    if kwargs is None:
        return {}
    mapping = _validate_mapping_type(kwargs, f"{ctx} kwargs")
    mapping = _validate_string_keys(mapping, f"{ctx} kwargs")
    result: dict[str, object] = {}
    for key, value in mapping.items():
        _validate_no_bytes(value, f"{ctx} kwargs values")
        result[key] = value
    return result


def _resolve_handler_class(name: str) -> object:
    """Return the builder class for ``name`` or raise ``ValueError``."""
    cls = _HANDLER_CLASS_MAP.get(name)
    if cls is None:
        msg = f"unsupported handler class {name!r}"
        raise ValueError(msg)
    return cls


def _validate_handler_keys(hid: str, data: Mapping[str, object]) -> None:
    """Validate that ``data`` contains only supported handler keys."""
    allowed = {"class", "level", "filters", "args", "kwargs", "formatter"}
    unknown = set(data.keys()) - allowed
    if unknown:
        msg = f"handler {hid!r} has unsupported keys: {sorted(unknown)!r}"
        raise ValueError(msg)


def _validate_handler_class(hid: str, cls_name: object) -> str:
    """Ensure a string handler class name is provided."""
    if not isinstance(cls_name, str):
        msg = f"handler {hid!r} missing class"
        raise TypeError(msg)
    return cls_name


def _validate_unsupported_features(data: Mapping[str, object]) -> None:
    """Reject handler features not yet implemented."""
    if "level" in data:
        msg = "handler level is not supported"
        raise ValueError(msg)
    if "filters" in data:
        msg = "handler filters are not supported"
        raise ValueError(msg)


def _validate_handler_config(
    hid: str, data: Mapping[str, object]
) -> tuple[str, list[object], dict[str, object], object | None]:
    """Validate handler ``data`` and return construction parameters."""
    _validate_handler_keys(hid, data)
    cls_name = _validate_handler_class(hid, data.get("class"))
    _validate_unsupported_features(data)
    ctx = f"handler {hid!r}"
    args = _coerce_args(data.get("args"), ctx)
    kwargs = _coerce_kwargs(data.get("kwargs"), ctx)
    return cls_name, args, kwargs, data.get("formatter")


def _create_handler_instance(
    hid: str, cls_name: str, args: list[object], kwargs: dict[str, object]
) -> object:
    """Instantiate a handler builder and wrap constructor errors."""
    builder_cls = _resolve_handler_class(cls_name)
    if builder_cls is SocketHandlerBuilder:
        from . import config_socket

        return config_socket._build_socket_handler_builder(hid, args, kwargs)
    try:
        args_t = tuple(args)
        kwargs_d = dict(kwargs)
        return cast("Any", builder_cls)(*args_t, **kwargs_d)  # pyright: ignore[reportCallIssue]
    except (TypeError, ValueError, HandlerConfigError, HandlerIOError) as exc:
        msg = f"failed to construct handler {hid!r}: {exc}"
        raise ValueError(msg) from exc


def _build_handler_from_dict(hid: str, data: Mapping[str, object]) -> object:
    """Create a handler builder from ``dictConfig`` handler data."""
    cls_name, args, kwargs, fmt = _validate_handler_config(hid, data)
    builder = cast("Any", _create_handler_instance(hid, cls_name, args, kwargs))
    if fmt is not None:
        if not isinstance(fmt, str):
            msg = "formatter must be a string"
            raise TypeError(msg)
        builder = builder.with_formatter(fmt)
    return builder


def _validate_logger_handlers(handlers_obj: object) -> list[str]:
    """Validate logger ``handlers`` list and return it."""
    if not isinstance(handlers_obj, (list, tuple)):
        msg = "logger handlers must be a list or tuple of strings"
        raise TypeError(msg)
    handlers_seq = cast("Sequence[object]", handlers_obj)
    if not all(isinstance(h, str) for h in handlers_seq):
        msg = "logger handlers must be a list or tuple of strings"
        raise TypeError(msg)
    return list(cast("Sequence[str]", handlers_seq))


def _validate_logger_config_keys(name: str, data: Mapping[str, object]) -> None:
    """Ensure ``data`` uses only supported logger keys."""
    allowed = {"level", "handlers", "propagate", "filters"}
    unknown = set(data.keys()) - allowed
    if unknown:
        msg = f"logger {name!r} has unsupported keys: {sorted(unknown)!r}"
        raise ValueError(msg)
    if "filters" in data:
        msg = "filters are not supported"
        raise ValueError(msg)


def _validate_propagate_value(value: object) -> bool:
    """Validate the ``propagate`` value for a logger."""
    if not isinstance(value, bool):
        msg = "logger propagate must be a bool"
        raise TypeError(msg)
    return value


def _build_logger_from_dict(name: str, data: Mapping[str, object]) -> object:
    """Create a ``LoggerConfigBuilder`` from ``dictConfig`` logger data."""
    _validate_logger_config_keys(name, data)
    builder = LoggerConfigBuilder()
    if "level" in data:
        builder = builder.with_level(data["level"])
    if "handlers" in data:
        handlers = _validate_logger_handlers(data["handlers"])
        builder = builder.with_handlers(handlers)
    if "propagate" in data:
        propagate = _validate_propagate_value(data["propagate"])
        builder = builder.with_propagate(propagate)
    return builder


def _validate_dict_config(config: Mapping[str, object]) -> int:
    """Validate top-level configuration and return the version."""
    if "incremental" in config:
        msg = "incremental configuration is not supported"
        raise ValueError(msg)
    version = int(cast("int", config.get("version", 1)))
    if version != 1:
        msg = f"unsupported configuration version {version}"
        raise ValueError(msg)
    if "filters" in config:
        msg = "filters are not supported"
        raise ValueError(msg)
    return version


def _create_config_builder(version: int, config: Mapping[str, object]) -> object:
    """Initialize a ``ConfigBuilder`` with global options."""
    cb = ConfigBuilder()
    builder = cb.with_version(version)
    if "disable_existing_loggers" in config:
        value = config["disable_existing_loggers"]
        if not isinstance(value, bool):
            msg = "disable_existing_loggers must be a bool"
            raise TypeError(msg)
        builder = builder.with_disable_existing_loggers(value)
    return builder


def _validate_formatter_field(
    fcfg: Mapping[str, object], field: str, field_type: str
) -> str | None:
    """Return the string value for ``field`` or ``None`` if absent."""
    if field not in fcfg:
        return None
    value = fcfg[field]
    if not isinstance(value, str):
        msg = f"formatter '{field_type}' must be a string"
        raise TypeError(msg)
    return value


def _build_formatter(fcfg: Mapping[str, object]) -> object:
    """Build a :class:`FormatterBuilder` from configuration."""
    allowed = {"format", "datefmt"}
    unknown = set(fcfg.keys()) - allowed
    if unknown:
        msg = f"formatter has unsupported keys: {sorted(unknown)!r}"
        raise ValueError(msg)
    fb = FormatterBuilder()
    fmt = _validate_formatter_field(fcfg, "format", "format")
    if fmt is not None:
        fb = fb.with_format(fmt)
    datefmt = _validate_formatter_field(fcfg, "datefmt", "datefmt")
    if datefmt is not None:
        fb = fb.with_datefmt(datefmt)
    return fb


def _validate_section_mapping(section: object, name: str) -> Mapping[object, object]:
    """Ensure a configuration ``section`` is a mapping."""
    return _validate_mapping_type(section, name)


def dictConfig(config: Mapping[str, object]) -> None:  # noqa: N802
    """Configure logging using a ``dictConfig``-style dictionary.

    Parameters
    ----------
    config : Mapping[str, object]
        A dictionary compatible with :mod:`logging.config`. Supported keys are
        ``version``, ``disable_existing_loggers``, ``formatters``, ``handlers``,
        ``loggers``, and ``root``. Unsupported features (e.g., ``filters``,
        handler ``level``) raise ``ValueError``.

    Raises
    ------
    ValueError
        If the configuration uses unsupported features or invalid schemas.

    Examples
    --------
    >>> dictConfig({
    ...     "version": 1,
    ...     "handlers": {"h": {"class": "femtologging.StreamHandler"}},
    ...     "root": {"level": "INFO", "handlers": ["h"]},
    ... })

    """
    from . import config_sections

    version = _validate_dict_config(config)
    builder = cast("Any", _create_config_builder(version, config))
    builder = config_sections._process_formatters(builder, config)
    builder = config_sections._process_handlers(builder, config)
    builder = config_sections._process_loggers(builder, config)
    builder = config_sections._process_root_logger(builder, config)
    builder.build_and_init()


__all__ = [
    "ConfigBuilder",
    "FileHandlerBuilder",
    "FormatterBuilder",
    "LevelFilterBuilder",
    "LoggerConfigBuilder",
    "NameFilterBuilder",
    "OverflowPolicy",
    "RotatingFileHandlerBuilder",
    "SocketHandlerBuilder",
    "StreamHandlerBuilder",
    "dictConfig",
]
