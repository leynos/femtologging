"""Socket handler configuration for femtologging.dictConfig.

This module handles the construction of :class:`SocketHandlerBuilder` instances
from ``dictConfig``-style configuration dictionaries, including TCP/Unix
transport configuration.

The entry point is :func:`_build_socket_handler_builder`, called from
:func:`femtologging.config._create_handler_instance` when constructing socket
handlers.

TLS and backoff configuration parsing is delegated to
:mod:`femtologging.config_socket_opts`.
"""

from __future__ import annotations

import collections.abc as cabc
import dataclasses
import typing as typ

from . import _femtologging_rs as rust
from .config_socket_opts import _pop_socket_backoff_kwargs, _pop_socket_tls_kwargs

Mapping = cabc.Mapping
Any = typ.Any
cast = typ.cast

rust = typ.cast("Any", rust)
SocketHandlerBuilder = rust.SocketHandlerBuilder
BackoffConfig = getattr(rust, "BackoffConfig", None)

TCP_ARG_COUNT = 2


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
    if not transport_configured:
        msg = f"handler {hid!r} socket requires host/port or unix_path"
        raise ValueError(msg)
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
    """Apply positional args to configure socket transport."""
    if not args:
        return builder, transport_configured
    if len(args) == TCP_ARG_COUNT:
        host, port = args
        _validate_host_port(
            hid, host, port, context="socket args must be (host: str, port: int)"
        )
        return builder.with_tcp(host, port), True
    if len(args) == 1:
        (path,) = args
        _validate_unix_path(hid, path)
        return builder.with_unix_path(path), True
    msg = (
        f"handler {hid!r} socket args must be either a {TCP_ARG_COUNT}-tuple "
        "of (host, port) or a single unix_path"
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
    """Apply keyword args to configure socket transport."""
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


_UINT_OPTION_METHODS: typ.Final[dict[str, str]] = {
    "capacity": "with_capacity",
    "connect_timeout_ms": "with_connect_timeout_ms",
    "write_timeout_ms": "with_write_timeout_ms",
    "max_frame_size": "with_max_frame_size",
}


def _apply_backoff_to_builder(
    builder: SocketHandlerBuilder,
    backoff_overrides: dict[str, int | None],
) -> SocketHandlerBuilder:
    """Apply backoff configuration to the builder."""
    if BackoffConfig is None:
        return builder.with_backoff(**backoff_overrides)
    return builder.with_backoff(BackoffConfig(backoff_overrides))


def _apply_socket_tuning_kwargs(
    hid: str,
    builder: SocketHandlerBuilder,
    kwargs: dict[str, object],
) -> SocketHandlerBuilder:
    """Apply tuning kwargs (capacity, timeouts, TLS, backoff) to the builder."""
    for option_name, method_name in _UINT_OPTION_METHODS.items():
        value = _pop_socket_uint(hid, kwargs, option_name)
        if value is None:
            continue
        builder = getattr(builder, method_name)(value)

    tls_config = _pop_socket_tls_kwargs(hid, kwargs)
    if tls_config is not None:
        domain, insecure = tls_config
        builder = builder.with_tls(domain, insecure=insecure)

    backoff_overrides = _pop_socket_backoff_kwargs(hid, kwargs)
    if backoff_overrides is not None:
        builder = _apply_backoff_to_builder(builder, backoff_overrides)

    return builder


def _pop_socket_uint(hid: str, kwargs: dict[str, object], key: str) -> int | None:
    """Pop and validate a non-negative integer kwarg."""
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


def _validate_transport_flag_type(hid: str, transport_flag: object) -> None:
    """Validate that transport flag is a string."""
    if not isinstance(transport_flag, str):
        msg = f"handler {hid!r} socket kwargs transport must be a string"
        raise TypeError(msg)


def _validate_transport_flag_value(hid: str, transport_flag: str) -> None:
    """Validate that transport flag value is 'tcp' or 'unix'."""
    if transport_flag.lower() not in {"tcp", "unix"}:
        msg = f"handler {hid!r} socket kwargs transport must be 'tcp' or 'unix'"
        raise ValueError(msg)


def _consume_socket_transport_flag(hid: str, kwargs: dict[str, object]) -> None:
    """Consume and validate the transport flag kwarg (documentation only)."""
    transport_flag = kwargs.pop("transport", None)
    if transport_flag is None:
        return
    _validate_transport_flag_type(hid, transport_flag)
    _validate_transport_flag_value(hid, cast("str", transport_flag))


def _apply_host_port_kwargs(
    hid: str,
    builder: SocketHandlerBuilder,
    transport_kw: _TransportKwargs,
    *,
    transport_configured: bool,
) -> tuple[SocketHandlerBuilder, bool]:
    """Apply host/port kwargs to configure TCP transport."""
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
    """Validate host/port kwargs for TCP transport configuration."""
    if transport_kw.unix_path is not None:
        msg = f"handler {hid!r} socket kwargs must not mix host/port with unix_path"
        raise ValueError(msg)
    if transport_kw.host is None or transport_kw.port is None:
        msg = f"handler {hid!r} socket kwargs require both host and port"
        raise ValueError(msg)
    if transport_configured:
        msg = f"handler {hid!r} socket transport already configured via args"
        raise ValueError(msg)
    _validate_host_port(
        hid,
        transport_kw.host,
        transport_kw.port,
        context="socket kwargs host must be str and port must be int",
    )


def _validate_host_port(hid: str, host: object, port: object, *, context: str) -> None:
    """Validate host and port types for socket handler configuration.

    Parameters
    ----------
    hid
        Handler identifier for error messages.
    host
        The host value to validate (must be str).
    port
        The port value to validate (must be int, not bool).
    context
        Error message context describing the validation failure.

    """
    msg = f"handler {hid!r} {context}"
    if not isinstance(host, str):
        raise TypeError(msg)
    # ``bool`` subclasses ``int`` so reject it explicitly before the integer check.
    if isinstance(port, bool) or not isinstance(port, int):
        raise TypeError(msg)


def _validate_unix_path(hid: str, path: object) -> None:
    """Validate a Unix socket path argument."""
    if not isinstance(path, str):
        msg = f"handler {hid!r} unix socket path must be a string"
        raise TypeError(msg)


def _ensure_no_extra_socket_kwargs(hid: str, kwargs: dict[str, object]) -> None:
    """Raise ValueError if any unrecognised kwargs remain."""
    if kwargs:
        msg = f"handler {hid!r} has unsupported socket kwargs: {sorted(kwargs)!r}"
        raise ValueError(msg)
