"""HTTP capture helpers shared by the native-handler context suites.

The recording server accepts one POST per delivered record and queues the raw
body for assertions. It is kept out of the test modules so each module can stay
focused on the context contract it exercises.
"""

from __future__ import annotations

import http.server
import queue
import threading
import typing as typ

if typ.TYPE_CHECKING:
    import socket


class HTTPContextCaptureHandler(http.server.BaseHTTPRequestHandler):
    """Capture the body posted by the HTTP handler under test."""

    def do_POST(self) -> None:
        """Store the request body and acknowledge the handler delivery."""
        size = int(self.headers["Content-Length"])
        payload = self.rfile.read(size)
        server = typ.cast("RecordingHTTPServer", self.server)
        self.send_response(200)
        self.send_header("Content-Length", "0")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True
        server.payloads.put(payload)

    @typ.override
    def log_message(
        self,
        format: str,
        *arguments: object,
    ) -> None:
        """Suppress expected local-server request logging during this test."""
        _ = (format, arguments)


class RecordingHTTPServer(http.server.ThreadingHTTPServer):
    """Threading HTTP server that queues native-handler request bodies."""

    allow_reuse_address = True

    def __init__(self) -> None:
        """Bind a loopback server with a payload queue and completion event."""
        super().__init__(("127.0.0.1", 0), HTTPContextCaptureHandler)
        self.payloads: queue.Queue[bytes] = queue.Queue()
        self.request_completed = threading.Event()

    def process_request_thread(
        self,
        request: socket.socket | tuple[bytes, socket.socket],
        client_address: object,
    ) -> None:
        """Signal after request handling and socket shutdown complete."""
        super().process_request_thread(request, client_address)
        self.request_completed.set()
