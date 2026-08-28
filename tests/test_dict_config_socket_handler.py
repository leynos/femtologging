"""Socket handler-specific tests for femtologging.dictConfig behaviour."""

from __future__ import annotations

import queue
import socketserver
import struct
import threading
import typing as typ

import pytest

import femtologging.config as config_module
import femtologging.config_socket as config_socket_module
from femtologging import (
    BackoffConfig,
    SocketHandlerBuilder,
    dictConfig,
    get_logger,
    reset_manager,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from femtologging._femtologging_rs import BackoffConfigDict


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    """Reset the global manager around each test."""
    reset_manager()
    yield
    reset_manager()


class _SocketCaptureHandler(socketserver.BaseRequestHandler):
    """Collect framed payloads emitted by FemtoSocketHandler."""

    def handle(self) -> None:
        length_bytes = self.request.recv(4)
        if not length_bytes:
            return
        length = struct.unpack(">I", length_bytes)[0]
        payload = self.request.recv(length)
        server = typ.cast("_SocketServer", self.server)
        server.queue.put(payload)


class _SocketServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True

    def __init__(self, address: tuple[str, int]) -> None:
        super().__init__(address, _SocketCaptureHandler)
        self.queue: queue.Queue[bytes] = queue.Queue()


@pytest.fixture(name="capture_server")
def fixture_capture_server() -> cabc.Iterator[_SocketServer]:
    """Serve a loopback TCP endpoint that captures framed payloads."""
    with _SocketServer(("127.0.0.1", 0)) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield server
        finally:
            server.shutdown()
            thread.join(timeout=1)
            assert not thread.is_alive(), (
                "the capture server thread must stop once shutdown() returns, "
                "otherwise later tests inherit a stray listener"
            )


def _build_socket_handler_from_kwargs(
    handler_id: str,
    kwargs: dict[str, object],
) -> object:
    """Build a socket handler builder from dictConfig-style kwargs."""
    return config_module._build_handler_from_dict(
        handler_id,
        {"class": "femtologging.SocketHandler", "kwargs": kwargs},
    )


def _assert_socket_builder_kwargs(
    builder: object,
    expected: dict[str, object],
    context: str,
) -> None:
    """Assert *builder* is a socket builder whose kwargs match *expected*."""
    assert isinstance(builder, SocketHandlerBuilder), (
        f"{context}: dictConfig must construct a SocketHandlerBuilder, but it "
        f"produced a {type(builder).__name__}"
    )
    actual = builder.as_dict()
    assert actual == expected, (
        f"{context}: the socket builder kwargs must match the requested "
        f"configuration; expected {expected}, got {actual}"
    )


def test_dict_config_socket_handler(capture_server: _SocketServer) -> None:
    """Ensure dictConfig wires a socket handler builder correctly."""
    host, port = capture_server.server_address[:2]

    cfg = {
        "version": 1,
        "handlers": {
            "sock": {
                "class": "logging.handlers.SocketHandler",
                "args": [str(host), int(port)],
            }
        },
        "root": {"level": "INFO", "handlers": ["sock"]},
    }

    dictConfig(cfg)
    get_logger("root").log("INFO", "message")

    payload = capture_server.queue.get(timeout=2)
    assert payload, (
        "the dictConfig-built socket handler must frame and send the record, "
        "but the server received an empty payload"
    )


def test_dict_config_socket_handler_round_trip_kwargs() -> None:
    """Support feeding ``SocketHandlerBuilder.as_dict()`` output back to dictConfig."""
    backoff_config: BackoffConfigDict = {
        "base_ms": 50,
        "cap_ms": 500,
        "reset_after_ms": 2000,
        "deadline_ms": 4000,
    }
    builder = (
        SocketHandlerBuilder()
        .with_tcp("127.0.0.1", 9020)
        .with_capacity(256)
        .with_connect_timeout_ms(750)
        .with_write_timeout_ms(1500)
        .with_max_frame_size(4096)
        .with_tls("example.com", insecure=True)
        .with_backoff(BackoffConfig(backoff_config))
    )
    expected_kwargs = builder.as_dict()
    round_trip = _build_socket_handler_from_kwargs("sock", dict(expected_kwargs))

    _assert_socket_builder_kwargs(
        round_trip,
        expected_kwargs,
        "SocketHandlerBuilder.as_dict() output fed back through dictConfig",
    )


def test_dict_config_socket_handler_backoff_legacy_kwargs(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Apply backoff overrides through the legacy kwargs path."""
    socket_handler_classes = (
        "logging.handlers.SocketHandler",
        "femtologging.SocketHandler",
        "femtologging.FemtoSocketHandler",
    )

    class LegacyBuilder:
        def __init__(self) -> None:
            self.host: str | None = None
            self.port: int | None = None
            self.overrides: dict[str, int | None] | None = None

        def with_tcp(self, host: str, port: int) -> LegacyBuilder:
            self.host = host
            self.port = port
            return self

        def with_backoff(self, **overrides: int | None) -> LegacyBuilder:
            self.overrides = dict(overrides)
            return self

    monkeypatch.setattr(config_socket_module, "BackoffConfig", None)
    monkeypatch.setattr(config_socket_module, "SocketHandlerBuilder", LegacyBuilder)
    monkeypatch.setattr(config_module, "SocketHandlerBuilder", LegacyBuilder)
    for handler_cls in socket_handler_classes:
        monkeypatch.setitem(
            config_module._HANDLER_CLASS_MAP, handler_cls, LegacyBuilder
        )

    nested_builder = _build_socket_handler_from_kwargs(
        "sock",
        {
            "host": "127.0.0.1",
            "port": 9023,
            "backoff": {
                "base_ms": 10,
                "cap_ms": 100,
                "reset_after_ms": None,
            },
        },
    )

    assert isinstance(nested_builder, LegacyBuilder), (
        "the patched handler class map must be honoured, but dictConfig built "
        f"a {type(nested_builder).__name__}"
    )
    assert (nested_builder.host, nested_builder.port) == ("127.0.0.1", 9023), (
        "the legacy path must forward host and port to with_tcp(), but got "
        f"{(nested_builder.host, nested_builder.port)}"
    )
    assert nested_builder.overrides == {
        "base_ms": 10,
        "cap_ms": 100,
        "reset_after_ms": None,
    }, (
        "without BackoffConfig the nested backoff mapping must be splatted as "
        f"with_backoff() keywords, but got {nested_builder.overrides}"
    )


def test_dict_config_socket_handler_accepts_nested_tls_backoff() -> None:
    """Accept structured TLS/backoff kwargs when constructing the socket builder."""
    backoff_config: BackoffConfigDict = {
        "base_ms": 10,
        "cap_ms": 100,
        "reset_after_ms": 200,
        "deadline_ms": 300,
    }
    tls_domain = "tls.example"
    tls_insecure = True
    tls_config = {"domain": tls_domain, "insecure": tls_insecure}
    socket_kwargs: dict[str, object] = {
        "host": "localhost",
        "port": 9021,
        "capacity": 128,
        "connect_timeout_ms": 250,
        "write_timeout_ms": 500,
        "max_frame_size": 2048,
        "tls": dict(tls_config),
        "backoff": dict(backoff_config),
    }
    nested_builder = _build_socket_handler_from_kwargs(
        "sock",
        socket_kwargs,
    )

    expected = (
        SocketHandlerBuilder()
        .with_tcp("localhost", 9021)
        .with_capacity(128)
        .with_connect_timeout_ms(250)
        .with_write_timeout_ms(500)
        .with_max_frame_size(2048)
        .with_tls(tls_domain, insecure=tls_insecure)
        .with_backoff(BackoffConfig(backoff_config))
    )
    _assert_socket_builder_kwargs(
        nested_builder,
        expected.as_dict(),
        "nested tls/backoff kwargs",
    )


def test_dict_config_socket_handler_rejects_conflicting_tls() -> None:
    """Reject configurations that disable TLS while providing TLS options."""
    with pytest.raises(
        ValueError, match="socket kwargs tls is disabled but TLS options were supplied"
    ):
        _build_socket_handler_from_kwargs(
            "sock",
            {
                "host": "127.0.0.1",
                "port": 9022,
                "tls": False,
                "tls_domain": "example.com",
            },
        )
