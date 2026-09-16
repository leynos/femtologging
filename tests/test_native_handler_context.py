"""Public-boundary coverage for context in Python-registered native handlers."""

from __future__ import annotations

import http.server
import queue
import threading
import time
import typing as typ

from femtologging import (
    FemtoHandler,
    FemtoLogger,
    FileHandlerBuilder,
    HTTPHandlerBuilder,
    RotatingFileHandlerBuilder,
    StreamHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
    log_context,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    import socket
    from pathlib import Path

_CONTEXT_OUTPUT = "correlation_id=abc123"


def _format_context(record: cabc.Mapping[str, object]) -> str:
    """Render the scoped correlation field from a native handler record."""
    metadata = typ.cast("cabc.Mapping[str, object]", record["metadata"])
    key_values = typ.cast("cabc.Mapping[str, str]", metadata["key_values"])
    return f"correlation_id={key_values['correlation_id']}"


def _emit_contextual_record(logger: FemtoLogger, handler: FemtoHandler) -> None:
    """Register *handler* and emit one record with scoped context."""
    logger.set_level("INFO")
    logger.add_handler(handler)
    with log_context(correlation_id="abc123"):
        logger.info("native handler context")


def _flush_after_delivery(logger: FemtoLogger, handler: FemtoHandler) -> None:
    """Flush both queue boundaries after an assertion observes delivery."""
    assert logger.flush_handlers(), "logger worker did not flush"
    assert handler.flush(), "native handler did not flush"


def test_python_registered_stream_handler_preserves_context() -> None:
    """A Python-registered stream handler must receive the original record."""
    rendered: queue.Queue[str] = queue.Queue()

    def capture(record: cabc.Mapping[str, object]) -> str:
        """Record the formatter output produced by the native stream handler."""
        output = _format_context(record)
        rendered.put(output)
        return output

    handler = StreamHandlerBuilder.stderr().with_formatter(capture).build()
    logger = FemtoLogger("native.context.stream")
    try:
        _emit_contextual_record(logger, handler)
        assert rendered.get(timeout=2) == _CONTEXT_OUTPUT, (
            "native stream handler output omitted the scoped correlation field"
        )
        _flush_after_delivery(logger, handler)
    finally:
        handler.close()
        logger.clear_handlers()


def _assert_file_handler_context(
    path: Path, handler: FemtoHandler, logger_name: str
) -> None:
    """Assert a native file handler persists a scoped field after registration."""
    logger = FemtoLogger(logger_name)
    try:
        _emit_contextual_record(logger, handler)
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline and _CONTEXT_OUTPUT not in path.read_text():
            time.sleep(0.01)
        assert _CONTEXT_OUTPUT in path.read_text(), (
            "native file handler output omitted the scoped correlation field"
        )
        _flush_after_delivery(logger, handler)
    finally:
        handler.close()
        logger.clear_handlers()


def test_python_registered_file_handler_preserves_context(tmp_path: Path) -> None:
    """A Python-registered file handler must persist scoped context."""
    path = tmp_path / "file.log"
    handler = FileHandlerBuilder(str(path)).with_formatter(_format_context).build()

    _assert_file_handler_context(path, handler, "native.context.file")


def test_python_registered_rotating_handler_preserves_context(tmp_path: Path) -> None:
    """A Python-registered rotating handler must persist scoped context."""
    path = tmp_path / "rotating.log"
    handler = (
        RotatingFileHandlerBuilder(str(path)).with_formatter(_format_context).build()
    )

    _assert_file_handler_context(path, handler, "native.context.rotating")


def test_python_registered_timed_rotating_handler_preserves_context(
    tmp_path: Path,
) -> None:
    """A Python-registered timed handler must persist scoped context."""
    path = tmp_path / "timed.log"
    handler = (
        TimedRotatingFileHandlerBuilder(str(path))
        .with_formatter(_format_context)
        .build()
    )

    _assert_file_handler_context(path, handler, "native.context.timed")


class _HTTPContextCaptureHandler(http.server.BaseHTTPRequestHandler):
    """Capture the body posted by the HTTP handler under test."""

    def do_POST(self) -> None:
        """Store the request body and acknowledge the handler delivery."""
        size = int(self.headers["Content-Length"])
        payload = self.rfile.read(size)
        server = typ.cast("_RecordingHTTPServer", self.server)
        self.send_response(200)
        self.send_header("Content-Length", "0")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True
        server.payloads.put(payload)

    def log_message(
        self,
        format: str,  # ruff: ignore[builtin-argument-shadowing] stdlib override requires the parameter name
        *arguments: object,
    ) -> None:
        """Suppress expected local-server request logging during this test."""
        _ = (format, arguments)


class _RecordingHTTPServer(http.server.ThreadingHTTPServer):
    """Threading HTTP server that queues native-handler request bodies."""

    allow_reuse_address = True

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), _HTTPContextCaptureHandler)
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


def test_python_registered_http_handler_preserves_context() -> None:
    """A Python-registered HTTP handler must serialize scoped context."""
    with _RecordingHTTPServer() as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        host, port = server.server_address[:2]
        handler = (
            HTTPHandlerBuilder()
            .with_endpoint(f"http://{host}:{port}/records", "POST")
            .build()
        )
        logger = FemtoLogger("native.context.http")
        try:
            _emit_contextual_record(logger, handler)
            assert b"correlation_id=abc123" in server.payloads.get(timeout=2), (
                "HTTP handler payload omitted the scoped correlation field"
            )
            assert server.request_completed.wait(timeout=2), (
                "HTTP request handler did not complete its response lifecycle"
            )
            _flush_after_delivery(logger, handler)
        finally:
            handler.close()
            logger.clear_handlers()
            server.shutdown()
            thread.join(timeout=1)
            assert not thread.is_alive(), "the HTTP server thread did not stop"
