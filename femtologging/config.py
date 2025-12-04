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
import dataclasses
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

TCP_ARG_COUNT = 2


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
        return _build_socket_handler_builder(hid, args, kwargs)
    try:
        args_t = tuple(args)
        kwargs_d = dict(kwargs)
        return cast("Any", builder_cls)(*args_t, **kwargs_d)  # pyright: ignore[reportCallIssue]
    except (TypeError, ValueError, HandlerConfigError, HandlerIOError) as exc:
        msg = f"failed to construct handler {hid!r}: {exc}"
        raise ValueError(msg) from exc


def _build_socket_handler_builder(
    hid: str, args: list[object], kwargs: dict[str, object]
) -> SocketHandlerBuilder:
    """Construct a ``SocketHandlerBuilder`` using fluent transport methods."""
    builder = SocketHandlerBuilder()
    transport_configured = False
    args_t = tuple(args)
    kwargs_d = dict(kwargs)

    builder, transport_configured = _apply_socket_args(
        hid,
        builder,
        args_t,
        transport_configured=transport_configured,
    )
    builder, transport_configured = _apply_socket_kwargs(
        hid,
        builder,
        kwargs_d,
        transport_configured=transport_configured,
    )
    builder = _apply_socket_tuning_kwargs(hid, builder, kwargs_d)
    _consume_socket_transport_flag(hid, kwargs_d)
    _ensure_no_extra_socket_kwargs(hid, kwargs_d)
    return builder


def _apply_socket_args(
    hid: str,
    builder: SocketHandlerBuilder,
    args: tuple[object, ...],
    *,
    transport_configured: bool,
) -> tuple[SocketHandlerBuilder, bool]:
    if not args:
        return builder, transport_configured
    if len(args) == TCP_ARG_COUNT:
        host, port = args
        _validate_host_port_args(hid, host, port)
        return builder.with_tcp(host, port), True
    if len(args) == 1:
        (path,) = args
        _validate_unix_path(hid, path)
        return builder.with_unix_path(path), True
    msg = (
        f"handler {hid!r} socket args must be either (host, port) or a single unix_path"
    )
    raise ValueError(msg)


@dataclasses.dataclass(slots=True)
class _TransportKwargs:
    """Transport-related keyword arguments for socket handler configuration."""

    host: object | None
    port: object | None
    unix_path: object | None


def _apply_socket_kwargs(
    hid: str,
    builder: SocketHandlerBuilder,
    kwargs: dict[str, object],
    *,
    transport_configured: bool,
) -> tuple[SocketHandlerBuilder, bool]:
    unix_kw = kwargs.pop("unix_path", None)
    if unix_kw is None:
        unix_kw = kwargs.pop("path", None)

    transport_kw = _TransportKwargs(
        host=kwargs.pop("host", None),
        port=kwargs.pop("port", None),
        unix_path=unix_kw,
    )

    builder, transport_configured = _apply_host_port_kwargs(
        hid,
        builder,
        transport_kw,
        transport_configured=transport_configured,
    )

    if transport_kw.unix_path is not None:
        _validate_unix_path(hid, transport_kw.unix_path)
        if transport_configured:
            msg = f"handler {hid!r} socket transport already configured via args"
            raise ValueError(msg)
        builder = builder.with_unix_path(transport_kw.unix_path)
        transport_configured = True

    return builder, transport_configured


def _apply_socket_tuning_kwargs(
    hid: str,
    builder: SocketHandlerBuilder,
    kwargs: dict[str, object],
) -> SocketHandlerBuilder:
    capacity = _pop_socket_uint(hid, kwargs, "capacity")
    if capacity is not None:
        builder = builder.with_capacity(capacity)

    connect_timeout = _pop_socket_uint(hid, kwargs, "connect_timeout_ms")
    if connect_timeout is not None:
        builder = builder.with_connect_timeout_ms(connect_timeout)

    write_timeout = _pop_socket_uint(hid, kwargs, "write_timeout_ms")
    if write_timeout is not None:
        builder = builder.with_write_timeout_ms(write_timeout)

    max_frame_size = _pop_socket_uint(hid, kwargs, "max_frame_size")
    if max_frame_size is not None:
        builder = builder.with_max_frame_size(max_frame_size)

    tls_config = _pop_socket_tls_kwargs(hid, kwargs)
    if tls_config is not None:
        domain, insecure = tls_config
        builder = builder.with_tls(domain, insecure=insecure)

    backoff_overrides = _pop_socket_backoff_kwargs(hid, kwargs)
    if backoff_overrides is not None:
        builder = builder.with_backoff(**backoff_overrides)

    return builder


def _pop_socket_uint(hid: str, kwargs: dict[str, object], key: str) -> int | None:
    if key not in kwargs:
        return None
    value = kwargs.pop(key)
    if isinstance(value, bool) or not isinstance(value, int):
        msg = f"handler {hid!r} socket kwargs {key} must be an int"
        raise TypeError(msg)
    if value < 0:
        msg = f"handler {hid!r} socket kwargs {key} must be non-negative"
        raise ValueError(msg)
    return value


def _pop_socket_tls_kwargs(
    hid: str, kwargs: dict[str, object]
) -> tuple[str | None, bool] | None:
    tls_value = kwargs.pop("tls", None)
    domain_kw = kwargs.pop("tls_domain", None)
    insecure_kw = kwargs.pop("tls_insecure", None)
    no_tls_config_provided = (
        tls_value is None and domain_kw is None and insecure_kw is None
    )
    if no_tls_config_provided:
        return None

    domain, insecure, enabled = _parse_tls_value(hid, tls_value)
    domain, enabled = _merge_tls_domain_kwarg(
        hid,
        domain,
        domain_kw,
        enabled=enabled,
    )
    insecure = _merge_tls_insecure_kwarg(
        hid,
        insecure_from_mapping=insecure,
        insecure_kw=insecure_kw,
    )
    if insecure_kw is not None:
        enabled = True

    if not enabled:
        return None

    _validate_tls_not_disabled(hid, tls_value)

    return domain, insecure


def _parse_tls_value(
    hid: str, tls_value: object
) -> tuple[str | None, bool | None, bool]:
    if isinstance(tls_value, Mapping):
        domain, insecure = _parse_tls_mapping(
            hid, cast("Mapping[object, object]", tls_value)
        )
        return domain, insecure, True
    if isinstance(tls_value, bool):
        return None, None, tls_value
    if tls_value is None:
        return None, None, False
    msg = f"handler {hid!r} socket kwargs tls must be a bool or mapping"
    raise TypeError(msg)


def _parse_tls_mapping(
    hid: str, tls_value: Mapping[object, object]
) -> tuple[str | None, bool | None]:
    mapping = _validate_mapping_type(tls_value, f"handler {hid!r} socket kwargs tls")
    mapping = _validate_string_keys(mapping, f"handler {hid!r} socket kwargs tls")
    unknown = set(mapping) - {"domain", "insecure"}
    if unknown:
        msg = (
            f"handler {hid!r} socket kwargs tls has unsupported keys: "
            f"{sorted(unknown)!r}"
        )
        raise ValueError(msg)
    domain = _extract_tls_domain_from_mapping(hid, mapping)
    insecure = _extract_tls_insecure_from_mapping(hid, mapping)
    return domain, insecure


def _extract_tls_domain_from_mapping(
    hid: str, tls_mapping: Mapping[str, object]
) -> str | None:
    if "domain" not in tls_mapping:
        return None
    domain = tls_mapping["domain"]
    if domain is not None and not isinstance(domain, str):
        msg = f"handler {hid!r} socket kwargs tls domain must be a string or None"
        raise TypeError(msg)
    return domain


def _extract_tls_insecure_from_mapping(
    hid: str, tls_mapping: Mapping[str, object]
) -> bool | None:
    if "insecure" not in tls_mapping:
        return None
    insecure_value = tls_mapping["insecure"]
    if not isinstance(insecure_value, bool):
        msg = f"handler {hid!r} socket kwargs tls insecure must be a bool"
        raise TypeError(msg)
    return insecure_value


def _merge_tls_domain_kwarg(
    hid: str,
    domain: str | None,
    domain_kw: object | None,
    *,
    enabled: bool,
) -> tuple[str | None, bool]:
    if domain_kw is None:
        return domain, enabled
    if not isinstance(domain_kw, str):
        msg = f"handler {hid!r} socket kwargs tls_domain must be a string or None"
        raise TypeError(msg)
    if domain is not None and domain_kw != domain:
        msg = f"handler {hid!r} socket kwargs tls has conflicting domain values"
        raise ValueError(msg)
    return domain_kw, True


def _merge_tls_insecure_kwarg(
    hid: str,
    *,
    insecure_from_mapping: bool | None,
    insecure_kw: object | None,
) -> bool:
    """Merge the tls_insecure kwarg with existing insecure value from mapping."""
    if insecure_kw is None:
        return insecure_from_mapping if insecure_from_mapping is not None else False
    if not isinstance(insecure_kw, bool):
        msg = f"handler {hid!r} socket kwargs tls_insecure must be a bool"
        raise TypeError(msg)
    if insecure_from_mapping is not None and insecure_kw != insecure_from_mapping:
        msg = f"handler {hid!r} socket kwargs tls has conflicting insecure values"
        raise ValueError(msg)
    return insecure_kw


def _validate_tls_not_disabled(hid: str, tls_value: object) -> None:
    if isinstance(tls_value, bool) and not tls_value:
        msg = (
            f"handler {hid!r} socket kwargs tls is disabled but TLS options were "
            "supplied"
        )
        raise ValueError(msg)


def _pop_socket_backoff_kwargs(
    hid: str, kwargs: dict[str, object]
) -> dict[str, int | None] | None:
    backoff_value = kwargs.pop("backoff", None)
    overrides: dict[str, int | None] = {}

    if backoff_value is not None:
        overrides = _extract_backoff_mapping_values(hid, backoff_value)

    overrides = _merge_backoff_alias_values(hid, kwargs, overrides)

    if not overrides:
        return None

    return overrides


def _extract_backoff_mapping_values(
    hid: str, backoff_value: object
) -> dict[str, int | None]:
    mapping = _validate_mapping_type(
        backoff_value, f"handler {hid!r} socket kwargs backoff"
    )
    mapping = _validate_string_keys(mapping, f"handler {hid!r} socket kwargs backoff")
    unknown = set(mapping) - {
        "base_ms",
        "cap_ms",
        "reset_after_ms",
        "deadline_ms",
    }
    if unknown:
        msg = (
            f"handler {hid!r} socket kwargs backoff has unsupported keys:"
            f" {sorted(unknown)!r}"
        )
        raise ValueError(msg)

    return {
        key: _extract_backoff_key(hid, key, mapping)
        for key in ("base_ms", "cap_ms", "reset_after_ms", "deadline_ms")
        if key in mapping
    }


def _extract_backoff_key(
    hid: str, key: str, mapping: Mapping[str, object]
) -> int | None:
    return _coerce_backoff_value(hid, key, mapping[key])


def _merge_backoff_alias_values(
    hid: str,
    kwargs: dict[str, object],
    overrides: dict[str, int | None],
) -> dict[str, int | None]:
    merged = dict(overrides)
    alias_map = {
        "backoff_base_ms": "base_ms",
        "backoff_cap_ms": "cap_ms",
        "backoff_reset_after_ms": "reset_after_ms",
        "backoff_deadline_ms": "deadline_ms",
    }
    for alias, target in alias_map.items():
        present = alias in kwargs
        value = _extract_backoff_alias(hid, kwargs, alias)
        if present or value is not None:
            existing = merged.get(target)
            _check_backoff_conflict(hid, target, existing, value)
            merged[target] = value
    return merged


def _extract_backoff_alias(
    hid: str,
    kwargs: dict[str, object],
    alias: str,
) -> int | None:
    """Extract and coerce a backoff alias kwarg, returning None if not present."""
    if alias not in kwargs:
        return None
    return _coerce_backoff_value(hid, alias, kwargs.pop(alias))


def _check_backoff_conflict(
    hid: str, target: str, existing: int | None, new: int | None
) -> None:
    """Raise ValueError if conflicting backoff values are detected."""
    if existing is None or new is None:
        return
    if existing == new:
        return
    msg = f"handler {hid!r} socket kwargs backoff {target} conflict"
    raise ValueError(msg)


def _coerce_backoff_value(hid: str, key: str, value: object) -> int | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, int):
        msg = f"handler {hid!r} socket kwargs {key} must be an int or None"
        raise TypeError(msg)
    if value < 0:
        msg = f"handler {hid!r} socket kwargs {key} must be non-negative"
        raise ValueError(msg)
    return value


def _consume_socket_transport_flag(hid: str, kwargs: dict[str, object]) -> None:
    transport_flag = kwargs.pop("transport", None)
    if transport_flag is None:
        return
    if not isinstance(transport_flag, str):
        msg = f"handler {hid!r} socket kwargs transport must be a string"
        raise TypeError(msg)
    if transport_flag.lower() not in {"tcp", "unix"}:
        msg = f"handler {hid!r} socket kwargs transport must be 'tcp' or 'unix'"
        raise ValueError(msg)


def _apply_host_port_kwargs(
    hid: str,
    builder: SocketHandlerBuilder,
    transport_kw: _TransportKwargs,
    *,
    transport_configured: bool,
) -> tuple[SocketHandlerBuilder, bool]:
    if transport_kw.host is None and transport_kw.port is None:
        return builder, transport_configured
    _validate_host_port_transport_kwargs(
        hid,
        transport_kw,
        transport_configured=transport_configured,
    )
    host = cast("str", transport_kw.host)
    port = cast("int", transport_kw.port)
    return builder.with_tcp(host, port), True


def _validate_host_port_transport_kwargs(
    hid: str,
    transport_kw: _TransportKwargs,
    *,
    transport_configured: bool,
) -> None:
    if transport_kw.unix_path is not None:
        msg = f"handler {hid!r} socket kwargs must not mix host/port with unix_path"
        raise ValueError(msg)
    if transport_kw.host is None or transport_kw.port is None:
        msg = f"handler {hid!r} socket kwargs require both host and port"
        raise ValueError(msg)
    _validate_host_port_kwargs(hid, transport_kw.host, transport_kw.port)
    if transport_configured:
        msg = f"handler {hid!r} socket transport already configured via args"
        raise ValueError(msg)


def _validate_host_port_args(hid: str, host: object, port: object) -> None:
    """Validate host and port arguments for socket handler."""
    if not isinstance(host, str):
        msg = f"handler {hid!r} socket args must be (host: str, port: int)"
        raise TypeError(msg)
    # ``bool`` subclasses ``int`` so reject it explicitly before the integer check.
    if isinstance(port, bool):
        msg = f"handler {hid!r} socket args must be (host: str, port: int)"
        raise TypeError(msg)
    if not isinstance(port, int):
        msg = f"handler {hid!r} socket args must be (host: str, port: int)"
        raise TypeError(msg)


def _validate_host_port_kwargs(hid: str, host: object, port: object) -> None:
    """Validate host and port keyword arguments for socket handler."""
    if not isinstance(host, str):
        msg = f"handler {hid!r} socket kwargs host must be str and port must be int"
        raise TypeError(msg)
    # ``bool`` subclasses ``int`` so reject it explicitly before the integer check.
    if isinstance(port, bool):
        msg = f"handler {hid!r} socket kwargs host must be str and port must be int"
        raise TypeError(msg)
    if not isinstance(port, int):
        msg = f"handler {hid!r} socket kwargs host must be str and port must be int"
        raise TypeError(msg)


def _validate_unix_path(hid: str, path: object) -> None:
    if not isinstance(path, str):
        msg = f"handler {hid!r} unix socket path must be a string"
        raise TypeError(msg)


def _ensure_no_extra_socket_kwargs(hid: str, kwargs: dict[str, object]) -> None:
    if kwargs:
        msg = f"handler {hid!r} has unsupported socket kwargs: {sorted(kwargs)!r}"
        raise ValueError(msg)


def _build_handler_from_dict(hid: str, data: Mapping[str, object]) -> object:
    """Create a handler builder from ``dictConfig`` handler data."""
    cls_name, args, kwargs, fmt = _validate_handler_config(hid, data)
    builder = cast("Any", _create_handler_instance(hid, cls_name, args, kwargs))
    if fmt is not None:
        if not isinstance(fmt, str):
            msg = "formatter must be a string"
            raise ValueError(msg)
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
            raise ValueError(msg)
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


def _validate_section_mapping(section: object, name: str) -> Mapping[str, object]:
    """Ensure a configuration ``section`` is a mapping."""
    return cast("Mapping[str, object]", _validate_mapping_type(section, name))


@dataclasses.dataclass(frozen=True)
class SectionProcessor:
    """Configuration for :func:`_process_config_section`."""

    section: str
    builder_method: str
    build_func: Callable[[str, Mapping[str, object]], object]
    err_tmpl: str | None = None


def _process_config_section(
    builder: Any, config: Mapping[str, object], processor: SectionProcessor
) -> None:
    """Process formatter, handler, and logger sections."""
    mapping = cast(
        "Mapping[object, object]",
        _validate_section_mapping(config.get(processor.section, {}), processor.section),
    )
    method = getattr(builder, processor.builder_method)
    for key, cfg in mapping.items():
        if not isinstance(key, str):
            if processor.err_tmpl is None:
                msg = f"{processor.section[:-1]} ids must be strings"
                raise TypeError(msg)
            raise TypeError(processor.err_tmpl.format(name=repr(key)))
        method(
            key,
            processor.build_func(
                key,
                _validate_section_mapping(cfg, f"{processor.section[:-1]} config"),
            ),
        )


def _process_formatters(builder: Any, config: Mapping[str, object]) -> None:
    """Attach formatter builders to ``builder``."""
    _process_config_section(
        builder,
        config,
        SectionProcessor(
            "formatters", "with_formatter", lambda fid, m: _build_formatter(m)
        ),
    )


def _process_handlers(builder: Any, config: Mapping[str, object]) -> None:
    """Attach handler builders to ``builder``."""
    _process_config_section(
        builder,
        config,
        SectionProcessor("handlers", "with_handler", _build_handler_from_dict),
    )


def _process_loggers(builder: Any, config: Mapping[str, object]) -> None:
    """Attach logger configurations to ``builder``."""
    _process_config_section(
        builder,
        config,
        SectionProcessor(
            "loggers",
            "with_logger",
            _build_logger_from_dict,
            err_tmpl="loggers section key {name} must be a string",
        ),
    )


def _process_root_logger(builder: Any, config: Mapping[str, object]) -> None:
    """Configure the root logger."""
    if "root" not in config:
        msg = "root logger configuration is required"
        raise ValueError(msg)
    root = config["root"]
    if not isinstance(root, Mapping):
        msg = "root logger configuration must be a mapping"
        raise TypeError(msg)
    builder.with_root_logger(
        _build_logger_from_dict("root", cast("Mapping[str, object]", root))
    )


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
    version = _validate_dict_config(config)
    builder = cast("Any", _create_config_builder(version, config))
    _process_formatters(builder, config)
    _process_handlers(builder, config)
    _process_loggers(builder, config)
    _process_root_logger(builder, config)
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
