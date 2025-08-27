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


def _evaluate_string_safely(value: str, context: str) -> object:
    """Safely evaluate a string ``value`` using ``ast.literal_eval``."""
    try:
        return ast.literal_eval(value)
    except (ValueError, SyntaxError) as exc:
        raise ValueError(f"invalid {context}: {value}") from exc


def _validate_mapping_type(value: object, name: str) -> Mapping[object, object]:
    """Ensure ``value`` is a mapping and not bytes-like."""
    if isinstance(value, (bytes, bytearray)) or not isinstance(value, Mapping):
        raise ValueError(f"{name} must be a mapping")
    return cast(Mapping[object, object], value)


def _validate_no_bytes(value: object, name: str) -> None:
    """Reject ``bytes`` or ``bytearray`` for ``value``."""
    if isinstance(value, (bytes, bytearray)):
        raise ValueError(f"{name} must not be bytes or bytearray")


def _validate_string_keys(
    mapping: Mapping[object, object], name: str
) -> Mapping[str, object]:
    """Ensure all keys in ``mapping`` are strings."""
    for key in mapping:
        if not isinstance(key, str):
            raise ValueError(f"{name} keys must be strings")
    return cast(Mapping[str, object], mapping)


def _coerce_args(args: object) -> list[object]:
    """Convert ``args`` into a list for handler construction."""
    if isinstance(args, str):
        args = _evaluate_string_safely(args, "args")
    if args is None:
        return []
    _validate_no_bytes(args, "args")
    if not isinstance(args, Sequence):
        raise ValueError("args must be a sequence")
    return list(args)


def _coerce_kwargs(kwargs: object) -> dict[str, object]:
    """Convert ``kwargs`` into a dictionary for handler construction."""
    if isinstance(kwargs, str):
        kwargs = _evaluate_string_safely(kwargs, "kwargs")
    if kwargs is None:
        return {}
    mapping = _validate_mapping_type(kwargs, "kwargs")
    mapping = _validate_string_keys(mapping, "kwargs")
    result: dict[str, object] = {}
    for key, value in mapping.items():
        _validate_no_bytes(value, "kwargs values")
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


def _validate_handler_keys(hid: str, data: Mapping[str, object]) -> None:
    """Validate that ``data`` contains only supported handler keys."""
    allowed = {"class", "level", "filters", "args", "kwargs", "formatter"}
    unknown = set(data.keys()) - allowed
    if unknown:
        raise ValueError(f"handler {hid!r} has unsupported keys: {sorted(unknown)!r}")


def _validate_handler_class(hid: str, cls_name: object) -> str:
    """Ensure a string handler class name is provided."""
    if not isinstance(cls_name, str):
        raise ValueError(f"handler {hid!r} missing class")
    return cls_name


def _validate_unsupported_features(data: Mapping[str, object]) -> None:
    """Reject handler features not yet implemented."""
    if data.get("level") is not None:
        raise ValueError("handler level is not supported")
    if data.get("filters"):
        raise ValueError("handler filters are not supported")


def _validate_handler_config(
    hid: str, data: Mapping[str, object]
) -> tuple[str, list[object], dict[str, object], object | None]:
    """Validate handler ``data`` and return construction parameters."""
    _validate_handler_keys(hid, data)
    cls_name = _validate_handler_class(hid, data.get("class"))
    args = _coerce_args(data.get("args"))
    kwargs = _coerce_kwargs(data.get("kwargs"))
    _validate_unsupported_features(data)
    return cls_name, args, kwargs, data.get("formatter")


def _create_handler_instance(
    hid: str, cls_name: str, args: list[object], kwargs: dict[str, object]
) -> FileHandlerBuilder | StreamHandlerBuilder:
    """Instantiate a handler builder and wrap constructor errors."""
    builder_cls = _resolve_handler_class(cls_name)
    try:
        return builder_cls(*args, **kwargs)
    except Exception as exc:  # pragma: no cover - constructor errors propagated
        raise ValueError(f"failed to construct handler {hid!r}: {exc}") from exc


def _build_handler_from_dict(
    hid: str, data: Mapping[str, object]
) -> FileHandlerBuilder | StreamHandlerBuilder:
    """Create a handler builder from ``dictConfig`` handler data."""
    cls_name, args, kwargs, fmt = _validate_handler_config(hid, data)
    builder = _create_handler_instance(hid, cls_name, args, kwargs)
    if fmt:
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


def _validate_section_mapping(section: object, name: str) -> Mapping[str, object]:
    """Ensure a configuration ``section`` is a mapping."""
    return cast(Mapping[str, object], _validate_mapping_type(section, name))


def _process_formatters(builder: ConfigBuilder, config: Mapping[str, object]) -> None:
    """Attach formatter builders to ``builder``."""
    for fid, fcfg in _validate_section_mapping(
        config.get("formatters", {}), "formatters"
    ).items():
        builder.with_formatter(
            fid, _build_formatter(_validate_section_mapping(fcfg, "formatter config"))
        )


def _process_handlers(builder: ConfigBuilder, config: Mapping[str, object]) -> None:
    """Attach handler builders to ``builder``."""
    for hid, hcfg in _validate_section_mapping(
        config.get("handlers", {}), "handlers"
    ).items():
        builder.with_handler(
            hid,
            _build_handler_from_dict(
                hid, _validate_section_mapping(hcfg, "handler config")
            ),
        )


def _process_loggers(builder: ConfigBuilder, config: Mapping[str, object]) -> None:
    """Attach logger configurations to ``builder``."""
    for name, lcfg in _validate_section_mapping(
        config.get("loggers", {}), "loggers"
    ).items():
        builder.with_logger(
            name,
            _build_logger_from_dict(
                name, _validate_section_mapping(lcfg, "logger config")
            ),
        )


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
