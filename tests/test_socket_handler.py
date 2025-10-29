"""Integration tests for the FemtoSocketHandler Python surface."""

from __future__ import annotations

import queue
import socketserver
import struct
import threading
from typing import cast

import femtologging


class _CaptureHandler(socketserver.BaseRequestHandler):
    """Capture framed payloads from the handler under test."""

    def handle(self) -> None:  # noqa: D401 - behaviour obvious from base class
        length_data = self.request.recv(4)
        if not length_data:
            return
        length = struct.unpack(">I", length_data)[0]
        payload = self.request.recv(length)
        server = cast(_RecordingTCPServer, self.server)
        server.queue.put(payload)


class _RecordingTCPServer(socketserver.ThreadingTCPServer):
    """Threading TCP server recording received payloads."""

    allow_reuse_address = True

    def __init__(self, server_address):
        super().__init__(server_address, _CaptureHandler)
        self.queue: queue.Queue[bytes] = queue.Queue()


def test_socket_handler_sends_records() -> None:
    """Verify the handler frames MessagePack payloads over TCP."""

    with _RecordingTCPServer(("127.0.0.1", 0)) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        host, port = server.server_address

        builder = femtologging.SocketHandlerBuilder(host, port)
        handler = builder.build()
        handler.handle("test.logger", "INFO", "message")

        payload = server.queue.get(timeout=2)
        assert payload, "payload should not be empty"

        handler.close()
        server.shutdown()
        thread.join(timeout=1)
