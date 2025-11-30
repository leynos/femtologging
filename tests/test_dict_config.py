"""Tests for femtologging.dictConfig integration and behaviour."""

from __future__ import annotations

from pathlib import Path
import time
from typing import cast

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

import queue
import socketserver
import struct
import threading

import femtologging.config as config_module
from femtologging import SocketHandlerBuilder, dictConfig, get_logger, reset_manager

scenarios("features/dict_config.feature")


class _SocketCaptureHandler(socketserver.BaseRequestHandler):
    """Collect framed payloads emitted by FemtoSocketHandler."""

    def handle(self) -> None:  # noqa: D401 - behaviour inherited from base class
        length_bytes = self.request.recv(4)
        if not length_bytes:
            return
        length = struct.unpack(">I", length_bytes)[0]
        payload = self.request.recv(length)
        server = cast(_SocketServer, self.server)
        server.queue.put(payload)


class _SocketServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True

    def __init__(self, address):
        super().__init__(address, _SocketCaptureHandler)
        self.queue: queue.Queue[bytes] = queue.Queue()


@given("the logging system is reset")
def reset_logging() -> None:
    reset_manager()


@when("I configure dictConfig with a stream handler")
def configure_dict_config() -> None:
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": "femtologging.StreamHandler"}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    dictConfig(cfg)


@then(parsers.parse('logging "{msg}" at "{level}" from root matches snapshot'))
def log_matches_snapshot(msg: str, level: str, snapshot) -> None:
    logger = get_logger("root")
    assert logger.log(level, msg) == snapshot


@then("calling dictConfig with incremental true raises ValueError")
def dict_config_incremental_fails() -> None:
    with pytest.raises(ValueError, match="incremental configuration is not supported"):
        dictConfig({"version": 1, "incremental": True, "root": {}})


@when(
    parsers.parse('I configure dictConfig with handler class "{cls}"'),
    target_fixture="config_error",
)
def configure_with_handler_class(cls: str) -> ValueError:
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": cls}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(ValueError) as exc:
        dictConfig(cfg)
    return exc.value


@then("dictConfig raises ValueError")
def dict_config_raises_value_error(config_error: ValueError) -> None:
    assert isinstance(config_error, ValueError)


def test_dict_config_file_handler_args_kwargs(tmp_path: Path) -> None:
    """Verify args and kwargs are evaluated for handler construction."""
    reset_manager()
    path = tmp_path / "out.log"
    cfg = {
        "version": 1,
        "handlers": {
            "f": {
                "class": "femtologging.FileHandler",
                "args": f"('{path}',)",
                "kwargs": "{}",
            }
        },
        "root": {"level": "INFO", "handlers": ["f"]},
    }
    dictConfig(cfg)
    logger = get_logger("root")
    logger.log("INFO", "file")
    contents = ""
    deadline = time.time() + 1.0
    while time.time() < deadline:
        if path.exists():
            contents = path.read_text()
            if "file" in contents:
                break
        time.sleep(0.01)
    else:
        pytest.fail("log file not written in time")
    assert "file" in contents


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
    """Support feeding ``SocketHandlerBuilder.as_dict()`` output back into dictConfig."""

    builder = (
        SocketHandlerBuilder()
        .with_tcp("127.0.0.1", 9020)
        .with_capacity(256)
        .with_connect_timeout_ms(750)
        .with_write_timeout_ms(1500)
        .with_max_frame_size(4096)
        .with_tls("example.com", insecure=True)
        .with_backoff(
            base_ms=50,
            cap_ms=500,
            reset_after_ms=2000,
            deadline_ms=4000,
        )
    )
    expected_kwargs = builder.as_dict()
    round_trip = config_module._build_handler_from_dict(
        "sock",
        {
            "class": "femtologging.SocketHandler",
            "kwargs": dict(expected_kwargs),
        },
    )

    assert isinstance(round_trip, SocketHandlerBuilder)
    assert round_trip.as_dict() == expected_kwargs


def test_dict_config_socket_handler_accepts_nested_tls_backoff() -> None:
    """Accept structured TLS/backoff kwargs when constructing the socket builder."""

    nested_builder = config_module._build_handler_from_dict(
        "sock",
        {
            "class": "femtologging.SocketHandler",
            "kwargs": {
                "host": "localhost",
                "port": 9021,
                "capacity": 128,
                "connect_timeout_ms": 250,
                "write_timeout_ms": 500,
                "max_frame_size": 2048,
                "tls": {"domain": "tls.example", "insecure": True},
                "backoff": {
                    "base_ms": 10,
                    "cap_ms": 100,
                    "reset_after_ms": 200,
                    "deadline_ms": 300,
                },
            },
        },
    )

    assert isinstance(nested_builder, SocketHandlerBuilder)
    settings = nested_builder.as_dict()
    assert settings["transport"] == "tcp"
    assert settings["host"] == "localhost"
    assert settings["port"] == 9021
    assert settings["capacity"] == 128
    assert settings["connect_timeout_ms"] == 250
    assert settings["write_timeout_ms"] == 500
    assert settings["max_frame_size"] == 2048
    assert settings["tls"] is True
    assert settings["tls_domain"] == "tls.example"
    assert settings["tls_insecure"] is True
    assert settings["backoff_base_ms"] == 10
    assert settings["backoff_cap_ms"] == 100
    assert settings["backoff_reset_after_ms"] == 200
    assert settings["backoff_deadline_ms"] == 300


def test_dict_config_socket_handler_rejects_conflicting_tls() -> None:
    """Reject configurations that disable TLS while providing TLS options."""

    with pytest.raises(
        ValueError, match="socket kwargs tls is disabled but TLS options were supplied"
    ):
        config_module._build_handler_from_dict(
            "sock",
            {
                "class": "femtologging.SocketHandler",
                "kwargs": {
                    "host": "127.0.0.1",
                    "port": 9022,
                    "tls": False,
                    "tls_domain": "example.com",
                },
            },
        )


@pytest.mark.parametrize(
    ("handler_config", "expected_error"),
    [
        ({"args": b"bytes"}, "handler 'h' args must not be bytes or bytearray"),
        (
            {"kwargs": {"path": b"oops"}},
            "handler 'h' kwargs values must not be bytes or bytearray",
        ),
        ({"args": 1}, "handler 'h' args must be a sequence"),
        ({"kwargs": []}, "handler 'h' kwargs must be a mapping"),
        ({"filters": []}, "handler filters are not supported"),
    ],
    ids=[
        "args-bytes",
        "kwargs-bytes",
        "args-type",
        "kwargs-type",
        "filters-unsupported",
    ],
)
def test_dict_config_handler_validation_errors(
    handler_config: dict[str, object],
    expected_error: str,
) -> None:
    """Test various handler validation errors in dictConfig."""
    reset_manager()
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": "femtologging.StreamHandler", **handler_config}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(ValueError, match=expected_error):
        dictConfig(cfg)


def test_dict_config_logger_filters_presence() -> None:
    reset_manager()
    cfg = {
        "version": 1,
        "loggers": {"a": {"filters": []}},
        "root": {"handlers": []},
    }
    with pytest.raises(ValueError, match="filters are not supported"):
        dictConfig(cfg)


@pytest.mark.parametrize(
    ("config", "msg"),
    [
        ({"version": 1}, r"root logger configuration is required"),
        ({"version": 2, "root": {}}, r"(unsupported|invalid).+version"),
        (
            {
                "version": 1,
                "handlers": {"h": {"class": "unknown"}},
                "root": {"handlers": ["h"]},
            },
            r"(unknown|unsupported).+handler class",
        ),
        ({"version": 1, "filters": {"f": {}}, "root": {}}, r"filters.+not supported"),
        (
            {
                "version": 1,
                "disable_existing_loggers": "yes",
                "root": {"handlers": []},
            },
            r"disable_existing_loggers must be a bool",
        ),
        (
            {"version": 1, "loggers": {1: {}}, "root": {"handlers": []}},
            r"loggers section key.+must be a string",
        ),
        (
            {
                "version": 1,
                "loggers": {"a": {"handlers": "h"}},
                "root": {"handlers": []},
            },
            r"logger handlers must be a list or tuple of strings",
        ),
        (
            {
                "version": 1,
                "loggers": {"a": {"propagate": "yes"}},
                "root": {"handlers": []},
            },
            r"logger propagate must be a bool",
        ),
        (
            {
                "version": 1,
                "formatters": {"f": {"format": 1}},
                "handlers": {
                    "h": {"class": "femtologging.StreamHandler", "formatter": "f"}
                },
                "root": {"handlers": ["h"]},
            },
            r"formatter 'format' must be a string",
        ),
        (
            {
                "version": 1,
                "handlers": {
                    "h": {"class": "femtologging.StreamHandler", "formatter": "x"}
                },
                "root": {"handlers": ["h"]},
            },
            r"unknown formatter id",
        ),
    ],
    ids=[
        "root-missing",
        "version-unsupported",
        "handler-class-unknown",
        "filters-unsupported",
        "disable-existing-loggers-type",
        "logger-id-type",
        "logger-handlers-type",
        "logger-propagate-type",
        "formatter-value-type",
        "formatter-id-unknown",
    ],
)
def test_dict_config_invalid_configs(config: dict[str, object], msg: str) -> None:
    """Invalid configurations raise ``ValueError``."""
    reset_manager()
    with pytest.raises(ValueError, match=msg):
        dictConfig(config)
