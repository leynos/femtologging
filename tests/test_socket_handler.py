"""Integration tests for the FemtoSocketHandler Python surface."""

from __future__ import annotations

import queue
import socketserver
import struct
import threading
import typing as typ

import pytest

import femtologging

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path


class _CaptureHandler(socketserver.BaseRequestHandler):
    """Capture framed payloads from the handler under test."""

    def handle(self) -> None:
        length_data = self.request.recv(4)
        if not length_data:
            return
        length = struct.unpack(">I", length_data)[0]
        payload = self.request.recv(length)
        server = typ.cast("_RecordingTCPServer", self.server)
        server.queue.put(payload)


class _RecordingTCPServer(socketserver.ThreadingTCPServer):
    """Threading TCP server recording received payloads."""

    allow_reuse_address = True

    def __init__(self, server_address: tuple[str, int]) -> None:
        super().__init__(server_address, _CaptureHandler)
        self.queue: queue.Queue[bytes] = queue.Queue()


@pytest.fixture(name="recording_server")
def fixture_recording_server() -> cabc.Iterator[_RecordingTCPServer]:
    """Serve a loopback TCP endpoint that records framed payloads."""
    with _RecordingTCPServer(("127.0.0.1", 0)) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield server
        finally:
            server.shutdown()
            thread.join(timeout=1)
            assert not thread.is_alive(), (
                "the recording server thread must stop once shutdown() returns, "
                "otherwise later tests inherit a stray listener"
            )


def test_socket_handler_sends_records(
    recording_server: _RecordingTCPServer,
) -> None:
    """Verify the handler frames MessagePack payloads over TCP."""
    host, port = recording_server.server_address[:2]

    handler = femtologging.SocketHandlerBuilder().with_tcp(str(host), int(port)).build()
    try:
        handler.handle("test.logger", "INFO", "message")
        payload = recording_server.queue.get(timeout=2)
    finally:
        handler.close()

    assert payload, (
        "the socket handler must frame and send the record, but the server "
        "received an empty payload"
    )

def test_python_registered_socket_handler_preserves_context() -> None:
    """A socket handler registered through Python must retain metadata fields."""
    with _RecordingTCPServer(("127.0.0.1", 0)) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        address = server.server_address
        handler = (
            femtologging
            .SocketHandlerBuilder()
            .with_tcp(str(address[0]), int(address[1]))
            .build()
        )
        logger = femtologging.FemtoLogger("socket.context")
        logger.set_level("INFO")
        logger.add_handler(handler)

        with femtologging.log_context(correlation_id="abc123"):
            logger.info("message")

        payload = server.queue.get(timeout=2)
        assert b"correlation_id" in payload
        assert b"abc123" in payload

        handler.close()
        server.shutdown()
        thread.join(timeout=1)
def test_socket_builder_tls_requires_tcp(tmp_path: Path) -> None:
    """TLS configuration must be rejected when no TCP transport is configured."""
    socket_path = tmp_path / "socket.sock"
    builder = femtologging.SocketHandlerBuilder().with_unix_path(str(socket_path))
    builder = builder.with_tls("example.com", insecure=False)

    with pytest.raises(femtologging.HandlerConfigError):
        builder.build()
