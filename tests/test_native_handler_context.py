"""Public-boundary coverage for context in Python-registered native handlers."""

from __future__ import annotations

import json
import queue
import threading
import time
import typing as typ

import pytest

from femtologging import (
    ConfigBuilder,
    FemtoHandler,
    FemtoLogger,
    FileHandlerBuilder,
    HTTPHandlerBuilder,
    LoggerConfigBuilder,
    PythonCallbackFilterBuilder,
    RotatingFileHandlerBuilder,
    StreamHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
    get_logger,
    log_context,
    reset_manager,
)
from tests.http_context_support import RecordingHTTPServer

if typ.TYPE_CHECKING:
    import collections.abc as cabc
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


def _emit_with_reserved_context(logger: FemtoLogger, route: str) -> None:
    """Emit one record whose reserved field must fail admission."""
    if route == "extra":
        logger.info("rejected", extra={"levelname": "CRITICAL"})
    else:
        with log_context(name="spoofed"):
            logger.info("rejected")


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


@pytest.mark.parametrize(
    ("factory", "filename", "logger_name"),
    [
        pytest.param(
            lambda path: (
                FileHandlerBuilder(str(path)).with_formatter(_format_context).build()
            ),
            "file.log",
            "native.context.file",
            id="file",
        ),
        pytest.param(
            lambda path: (
                RotatingFileHandlerBuilder(str(path))
                .with_formatter(_format_context)
                .build()
            ),
            "rotating.log",
            "native.context.rotating",
            id="rotating",
        ),
        pytest.param(
            lambda path: (
                TimedRotatingFileHandlerBuilder(str(path))
                .with_formatter(_format_context)
                .build()
            ),
            "timed.log",
            "native.context.timed",
            id="timed-rotating",
        ),
    ],
)
def test_python_registered_file_handlers_preserve_context(
    tmp_path: Path,
    factory: cabc.Callable[[Path], FemtoHandler],
    filename: str,
    logger_name: str,
) -> None:
    """Python-registered file-handler variants must persist scoped context."""
    path = tmp_path / filename
    handler = factory(path)

    _assert_file_handler_context(path, handler, logger_name)


def test_python_registered_http_handler_preserves_context() -> None:
    """A Python-registered HTTP handler must serialize scoped context."""
    with RecordingHTTPServer() as server:
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


def test_http_json_keeps_custom_fields_at_top_level() -> None:
    """JSON output keeps legitimate custom fields flat and string-valued."""
    with RecordingHTTPServer() as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        host, port = server.server_address[:2]
        handler = (
            HTTPHandlerBuilder()
            .with_endpoint(f"http://{host}:{port}/records", "POST")
            .with_json_format()
            .build()
        )
        logger = FemtoLogger("native.context.http.json")
        try:
            logger.set_level("INFO")
            logger.add_handler(handler)
            logger.info(
                "json context",
                extra={"correlation_id": "abc123", "attempt": 2},
            )
            payload = json.loads(server.payloads.get(timeout=2))
            assert payload["correlation_id"] == "abc123", (
                "custom fields should round-trip at the top level"
            )
            assert isinstance(payload["correlation_id"], str), (
                "custom JSON fields must remain string-valued"
            )
            assert payload["attempt"] == "2", (
                "non-string scalars should be stringified before serialization"
            )
            assert payload["name"] == "native.context.http.json", (
                "authoritative name must not be replaced by metadata"
            )
            assert payload["levelname"] == "INFO", (
                "authoritative levelname must be reported"
            )
            assert payload["msg"] == "json context", (
                "authoritative msg must be reported"
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


def test_http_record_fields_project_custom_fields_at_top_level() -> None:
    """``with_record_fields`` selects custom fields by their top-level wire name."""
    with RecordingHTTPServer() as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        host, port = server.server_address[:2]
        handler = (
            HTTPHandlerBuilder()
            .with_endpoint(f"http://{host}:{port}/records", "POST")
            .with_json_format()
            .with_record_fields(["name", "correlation_id"])
            .build()
        )
        logger = FemtoLogger("native.context.http.projected")
        try:
            logger.set_level("INFO")
            logger.add_handler(handler)
            logger.info(
                "projected context",
                extra={"correlation_id": "abc123", "attempt": 2},
            )
            payload = json.loads(server.payloads.get(timeout=2))
            assert set(payload) == {"name", "correlation_id"}, (
                "record_fields should emit exactly the requested fields"
            )
            assert payload["name"] == "native.context.http.projected", (
                "the requested built-in name should be projected"
            )
            assert payload["correlation_id"] == "abc123", (
                "the requested custom field should be projected by wire name"
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


@pytest.mark.parametrize("route", ["extra", "log_context"])
def test_reserved_context_is_rejected_before_http_delivery(*, route: str) -> None:
    """Reserved fields fail admission and never reach the HTTP endpoint."""
    with RecordingHTTPServer() as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        host, port = server.server_address[:2]
        handler = (
            HTTPHandlerBuilder()
            .with_endpoint(f"http://{host}:{port}/records", "POST")
            .build()
        )
        logger = FemtoLogger(f"native.context.http.reject.{route}")
        try:
            logger.set_level("INFO")
            logger.add_handler(handler)
            with pytest.raises(ValueError, match="reserved"):
                _emit_with_reserved_context(logger, route)
            logger.info("sentinel")
            payload = server.payloads.get(timeout=2)
            assert b"sentinel" in payload, "the queued sentinel record should arrive"
            assert server.payloads.empty(), (
                "a rejected record must not reach the HTTP endpoint"
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


def test_configured_http_handler_filters_propagated_records() -> None:
    """A configured HTTP filter suppresses rejected child-logger records."""
    reset_manager()

    def accept_record(record: object) -> bool:
        """Allow only the accepted message through the shared HTTP handler."""
        attributes = vars(record)
        if attributes["msg"] != "accepted":
            return False
        attributes["correlation_id"] = "REQ-99"
        return True

    with RecordingHTTPServer() as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        host, port = server.server_address[:2]
        try:
            (
                ConfigBuilder()
                .with_filter("accept", PythonCallbackFilterBuilder(accept_record))
                .with_handler(
                    "network",
                    HTTPHandlerBuilder()
                    .with_endpoint(f"http://{host}:{port}/records", "POST")
                    .with_filters(["accept"]),
                )
                .with_root_logger(
                    LoggerConfigBuilder().with_level("INFO").with_handlers(["network"])
                )
                .build_and_init()
            )
            child = get_logger("network.http.child")
            child.info("rejected")
            child.info("accepted")
            assert get_logger("root").flush_handlers(), "HTTP delivery should flush"
            payload = server.payloads.get(timeout=2)
            assert b"accepted" in payload, (
                "accepted child record should reach the HTTP endpoint"
            )
            assert b"rejected" not in payload, (
                "rejected child records must not reach the HTTP endpoint"
            )
            assert b"correlation_id=REQ-99" in payload, (
                "accepted filter enrichment should reach the HTTP endpoint"
            )
            assert server.payloads.empty(), "the rejected record must not be sent"
        finally:
            reset_manager()
            server.shutdown()
            thread.join(timeout=1)
            assert not thread.is_alive(), "the HTTP server thread did not stop"


def test_http_handler_rejects_unknown_filter_ids() -> None:
    """Configuration rejects an unresolved HTTP handler filter ID."""
    reset_manager()
    handler = (
        HTTPHandlerBuilder()
        .with_endpoint("http://127.0.0.1:1/records", "POST")
        .with_filters(["missing"])
    )
    config = (
        ConfigBuilder()
        .with_handler("network", handler)
        .with_root_logger(LoggerConfigBuilder().with_handlers(["network"]))
    )

    try:
        with pytest.raises(KeyError, match="missing"):
            config.build_and_init()
    finally:
        reset_manager()
