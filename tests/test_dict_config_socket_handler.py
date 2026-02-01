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
    from femtologging._femtologging_rs import BackoffConfigDict


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


def _build_socket_handler_from_kwargs(
    handler_id: str,
    kwargs: dict[str, object],
) -> object:
    """Build a socket handler builder from dictConfig-style kwargs."""
    return config_module._build_handler_from_dict(
        handler_id,
        {"class": "femtologging.SocketHandler", "kwargs": kwargs},
    )


def test_dict_config_socket_handler() -> None:
    """Ensure dictConfig wires a socket handler builder correctly."""
    reset_manager()
    with _SocketServer(("127.0.0.1", 0)) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        address = server.server_address
        host = str(address[0])
        port = int(address[1])

        cfg = {
            "version": 1,
            "handlers": {
                "sock": {
                    "class": "logging.handlers.SocketHandler",
                    "args": [host, port],
                }
            },
            "root": {"level": "INFO", "handlers": ["sock"]},
        }

        dictConfig(cfg)
        logger = get_logger("root")
        logger.log("INFO", "message")

        payload = server.queue.get(timeout=2)
        assert payload, "socket handler should emit payload"

        server.shutdown()
        thread.join(timeout=1)
        if thread.is_alive():
            pytest.fail("server thread did not terminate within timeout")


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

    assert isinstance(round_trip, SocketHandlerBuilder)
    assert round_trip.as_dict() == expected_kwargs


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

    assert isinstance(nested_builder, LegacyBuilder)
    assert nested_builder.host == "127.0.0.1"
    assert nested_builder.port == 9023
    assert nested_builder.overrides == {
        "base_ms": 10,
        "cap_ms": 100,
        "reset_after_ms": None,
    }


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

    assert isinstance(nested_builder, SocketHandlerBuilder)
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
    assert nested_builder.as_dict() == expected.as_dict()


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
