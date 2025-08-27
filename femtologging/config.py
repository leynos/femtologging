"""Dictionary-based logging configuration helpers."""

from __future__ import annotations

import ast
from typing import Final, Mapping, Sequence, cast

from . import _femtologging_rs as rust  # type: ignore[attr-defined]
from .overflow_policy import OverflowPolicy

StreamHandlerBuilder = rust.StreamHandlerBuilder  # type: ignore[attr-defined]
FileHandlerBuilder = rust.FileHandlerBuilder  # type: ignore[attr-defined]
ConfigBuilder = rust.ConfigBuilder  # type: ignore[attr-defined]
LoggerConfigBuilder = rust.LoggerConfigBuilder  # type: ignore[attr-defined]
FormatterBuilder = rust.FormatterBuilder  # type: ignore[attr-defined]


_HANDLER_CLASS_MAP: Final[
    dict[str, type[StreamHandlerBuilder] | type[FileHandlerBuilder]]
] = {
    "logging.StreamHandler": StreamHandlerBuilder,
    "femtologging.StreamHandler": StreamHandlerBuilder,
    "logging.FileHandler": FileHandlerBuilder,
    "femtologging.FileHandler": FileHandlerBuilder,
}


def _evaluate_string_args(args: str) -> object:
    """Safely evaluate string args using ``ast.literal_eval``."""
    try:
        return ast.literal_eval(args)
    except (ValueError, SyntaxError) as exc:
        raise ValueError(f"invalid args: {args}") from exc


def _validate_args_type(args: object) -> None:
    """Validate that ``args`` is a sequence and not bytes-like."""
    if isinstance(args, (bytes, bytearray)):
        raise ValueError("args must not be bytes or bytearray")
    if not isinstance(args, Sequence):
        raise ValueError("args must be a sequence")


def _coerce_args(args: object) -> list[object]:
    """Convert ``args`` into a list for handler construction."""
    if isinstance(args, str):
        args = _evaluate_string_args(args)
    if args is None:
        return []
    _validate_args_type(args)
    return list(args)


def _coerce_kwargs(kwargs: object) -> dict[str, object]:
    """Convert ``kwargs`` into a dictionary for handler construction."""
    if isinstance(kwargs, str):
        try:
            kwargs = ast.literal_eval(kwargs)
        except (ValueError, SyntaxError) as exc:
            raise ValueError(f"invalid kwargs: {kwargs}") from exc
    if kwargs is None:
        return {}
    if isinstance(kwargs, (bytes, bytearray)) or not isinstance(kwargs, Mapping):
        raise ValueError("kwargs must be a mapping")
    result: dict[str, object] = {}
    for key, value in kwargs.items():
        if not isinstance(key, str):
            raise ValueError("kwargs keys must be strings")
        if isinstance(value, (bytes, bytearray)):
            raise ValueError("kwargs values must not be bytes or bytearray")
        result[key] = value
    return result


def _resolve_handler_class(
    name: str,
) -> type[StreamHandlerBuilder] | type[FileHandlerBuilder]:
    """Return the builder class for ``name`` or raise ``ValueError``."""
    cls = _HANDLER_CLASS_MAP.get(name)
    if cls is None:
        raise ValueError(f"unsupported handler class {name!r}")
    return cls


def _build_handler_from_dict(
    hid: str, data: Mapping[str, object]
) -> FileHandlerBuilder | StreamHandlerBuilder:
    """Create a handler builder from ``dictConfig`` handler data."""
    allowed_keys = {"class", "level", "filters", "args", "kwargs", "formatter"}
    unknown = set(data.keys()) - allowed_keys
    if unknown:
        raise ValueError(f"handler {hid!r} has unsupported keys: {sorted(unknown)!r}")
    cls_name = data.get("class")
    if not isinstance(cls_name, str):
        raise ValueError(f"handler {hid!r} missing class")
    builder_cls = _resolve_handler_class(cls_name)
    args = _coerce_args(data.get("args"))
    kwargs = _coerce_kwargs(data.get("kwargs"))
    try:
        builder = builder_cls(*args, **kwargs)
    except Exception as exc:  # pragma: no cover - constructor errors propagated
        raise ValueError(f"failed to construct handler {hid!r}: {exc}") from exc
    if data.get("level") is not None:
        raise ValueError("handler level is not supported")
    if data.get("filters"):
        raise ValueError("handler filters are not supported")
    if fmt := data.get("formatter"):
        builder = builder.with_formatter(fmt)
    return builder


def _build_logger_from_dict(
    name: str, data: Mapping[str, object]
) -> LoggerConfigBuilder:
    """Create a ``LoggerConfigBuilder`` from ``dictConfig`` logger data."""
    if data.get("filters"):
        raise ValueError("filters are not supported")
    builder = LoggerConfigBuilder()
    if lvl := data.get("level"):
        builder = builder.with_level(lvl)
    if handlers := data.get("handlers"):
        builder = builder.with_handlers(handlers)
    if prop := data.get("propagate"):
        builder = builder.with_propagate(bool(prop))
    return builder


def _validate_dict_config(config: Mapping[str, object]) -> int:
    """Validate top-level configuration and return the version."""
    if config.get("incremental"):
        raise ValueError("incremental configuration is not supported")
    version = int(config.get("version", 1))
    if version != 1:
        raise ValueError(f"unsupported configuration version {version}")
    if config.get("filters"):
        raise ValueError("filters are not supported")
    return version


def _create_config_builder(version: int, config: Mapping[str, object]) -> ConfigBuilder:
    """Initialise a ``ConfigBuilder`` with global options."""
    builder = ConfigBuilder().with_version(version)
    if config.get("disable_existing_loggers"):
        builder = builder.with_disable_existing_loggers(True)
    return builder


def _build_formatter(fcfg: Mapping[str, object]) -> FormatterBuilder:
    """Build a :class:`FormatterBuilder` from configuration."""
    fb = FormatterBuilder()
    if fmt := fcfg.get("format"):
        fb = fb.with_format(fmt)
    if datefmt := fcfg.get("datefmt"):
        fb = fb.with_datefmt(datefmt)
    return fb


def _process_formatters(builder: ConfigBuilder, config: Mapping[str, object]) -> None:
    """Attach formatter builders to ``builder``."""
    formatters = config.get("formatters", {})
    if not isinstance(formatters, Mapping):
        raise ValueError("formatters must be a mapping")
    for fid, fcfg in formatters.items():
        if not isinstance(fcfg, Mapping):
            raise ValueError("formatter config must be a mapping")
        builder.with_formatter(fid, _build_formatter(fcfg))


def _process_handlers(builder: ConfigBuilder, config: Mapping[str, object]) -> None:
    """Attach handler builders to ``builder``."""
    handlers = config.get("handlers", {})
    if not isinstance(handlers, Mapping):
        raise ValueError("handlers must be a mapping")
    for hid, hcfg in handlers.items():
        if not isinstance(hcfg, Mapping):
            raise ValueError("handler config must be a mapping")
        builder.with_handler(hid, _build_handler_from_dict(hid, hcfg))


def _process_loggers(builder: ConfigBuilder, config: Mapping[str, object]) -> None:
    """Attach logger configurations to ``builder``."""
    loggers = config.get("loggers", {})
    if not isinstance(loggers, Mapping):
        raise ValueError("loggers must be a mapping")
    for name, lcfg in loggers.items():
        if not isinstance(lcfg, Mapping):
            raise ValueError("logger config must be a mapping")
        builder.with_logger(name, _build_logger_from_dict(name, lcfg))


def _process_root_logger(builder: ConfigBuilder, config: Mapping[str, object]) -> None:
    """Configure the root logger."""
    root = config.get("root")
    if not isinstance(root, Mapping):
        raise ValueError("root logger configuration must be a mapping")
    builder.with_root_logger(
        _build_logger_from_dict("root", cast(Mapping[str, object], root))
    )


def dictConfig(config: Mapping[str, object]) -> None:
    """Configure logging using a ``dictConfig``‑style dictionary.

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
    """

    version = _validate_dict_config(config)
    builder = _create_config_builder(version, config)
    _process_formatters(builder, config)
    _process_handlers(builder, config)
    _process_loggers(builder, config)
    _process_root_logger(builder, config)
    builder.build_and_init()


__all__ = [
    "ConfigBuilder",
    "LoggerConfigBuilder",
    "FormatterBuilder",
    "StreamHandlerBuilder",
    "FileHandlerBuilder",
    "dictConfig",
    "OverflowPolicy",
]
