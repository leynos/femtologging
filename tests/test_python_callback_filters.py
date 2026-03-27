"""Tests for stdlib-compatible Python callback filters."""

from __future__ import annotations

import contextvars
import io
import logging
import threading
import time
import typing as typ

import pytest

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    PythonCallbackFilterBuilder,
    StdlibHandlerAdapter,
    dictConfig,
    get_logger,
    reset_manager,
)

_REQUEST_ID: contextvars.ContextVar[str] = contextvars.ContextVar(
    "request_id", default=""
)


class RecordCollector:
    """Collect full record payloads from ``handle_record``."""

    def __init__(self) -> None:
        """Initialize the collector state."""
        self.records: list[dict[str, typ.Any]] = []
        self._lock = threading.Lock()

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback handler required by femtologging validation."""

    def handle_record(self, record: dict[str, typ.Any]) -> None:
        """Store structured records for assertions."""
        with self._lock:
            self.records.append(record)


class PrefixRejectingFilter(logging.Filter):
    """Example stdlib-style filter object with a ``filter`` method."""

    def __init__(self, prefix: str) -> None:
        """Store the accepted logger-name prefix."""
        super().__init__()
        self.prefix = prefix

    def filter(self, record: logging.LogRecord) -> bool:
        """Allow only records whose logger starts with ``self.prefix``."""
        return record.name.startswith(self.prefix)


class ContextFilterFactory(logging.Filter):
    """Factory used by ``dictConfig`` filter tests."""

    def __init__(self, request_id: str) -> None:
        """Store the request ID to inject into accepted records."""
        super().__init__()
        self.request_id = request_id

    def filter(self, record: logging.LogRecord) -> bool:
        """Attach the configured request ID and accept the record."""
        record.request_id = self.request_id
        return True


@pytest.fixture(autouse=True)
def reset_logger_state() -> typ.Iterator[None]:
    """Reset the global logging manager around each test."""
    reset_manager()
    yield
    reset_manager()


def _build_filtered_logger(filter_obj: object) -> RecordCollector:
    collector = RecordCollector()
    (
        ConfigBuilder()
        .with_filter("py", filter_obj)
        .with_logger("app", LoggerConfigBuilder().with_filters(["py"]))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
        .build_and_init()
    )
    get_logger("app").add_handler(collector)
    return collector


def _wait_for(condition: typ.Callable[[], bool], timeout: float = 1.0) -> None:
    """Poll ``condition`` until it becomes true or timeout expires."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if condition():
            return
        time.sleep(0.01)
    pytest.fail("condition was not met before timeout")


def test_python_callback_filter_builder_persists_contextvar_enrichment() -> None:
    """Accepted callback filters should persist new record attributes."""

    def enrich(record: logging.LogRecord) -> bool:
        record.correlation_id = _REQUEST_ID.get()
        return True

    collector = _build_filtered_logger(PythonCallbackFilterBuilder(enrich))
    _REQUEST_ID.set("req-123")

    logger = get_logger("app")
    assert logger.log("INFO", "hello") is not None
    _wait_for(lambda: len(collector.records) == 1)

    record = collector.records[0]
    assert record["metadata"]["key_values"]["correlation_id"] == "req-123"


def test_python_filter_object_rejects_records() -> None:
    """Objects exposing ``filter(record)`` should participate in filtering."""
    _build_filtered_logger(PrefixRejectingFilter("svc"))

    logger = get_logger("app")
    assert logger.log("INFO", "blocked") is None


def test_stdlib_handler_adapter_receives_enrichment_fields() -> None:
    """Enrichment should be visible to stdlib formatters via the adapter."""

    def enrich(record: logging.LogRecord) -> bool:
        record.request_id = "abc-123"
        return True

    (
        ConfigBuilder()
        .with_filter("py", enrich)
        .with_logger("app", LoggerConfigBuilder().with_filters(["py"]))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
        .build_and_init()
    )

    stream = io.StringIO()
    handler = logging.StreamHandler(stream)
    handler.setFormatter(logging.Formatter("%(request_id)s %(message)s"))
    get_logger("app").add_handler(StdlibHandlerAdapter(handler))

    assert get_logger("app").log("INFO", "hello") is not None
    _wait_for(lambda: stream.getvalue().strip() == "abc-123 hello")
    assert stream.getvalue().strip() == "abc-123 hello"


def test_dict_config_filter_factory_form_supports_kwargs() -> None:
    """Factory-mode filter entries should resolve and instantiate callbacks."""
    cfg = {
        "version": 1,
        "filters": {
            "factory": {
                "()": "tests.test_python_callback_filters.ContextFilterFactory",
                "request_id": "factory-123",
            }
        },
        "loggers": {"app": {"filters": ["factory"]}},
        "root": {"level": "DEBUG"},
    }
    dictConfig(cfg)

    collector = RecordCollector()
    get_logger("app").add_handler(collector)

    assert get_logger("app").log("INFO", "hello") is not None
    _wait_for(lambda: len(collector.records) == 1)
    assert collector.records[0]["metadata"]["key_values"]["request_id"] == "factory-123"


@pytest.mark.parametrize(
    "filter_cfg",
    [
        {
            "()": "tests.test_python_callback_filters.ContextFilterFactory",
            "level": "INFO",
        },
        {
            "()": "tests.test_python_callback_filters.ContextFilterFactory",
            "name": "app",
        },
    ],
)
def test_dict_config_rejects_mixed_factory_and_declarative_forms(
    filter_cfg: dict[str, object],
) -> None:
    """Factory and declarative filter forms must remain mutually exclusive."""
    cfg = {"version": 1, "filters": {"f": filter_cfg}, "root": {"level": "DEBUG"}}
    with pytest.raises(
        ValueError, match="must not mix '\\(\\)' with 'level' or 'name'"
    ):
        dictConfig(cfg)


def test_contextvar_enrichment_is_thread_local() -> None:
    """Concurrent callback filters should preserve per-thread contextvars."""

    def enrich(record: logging.LogRecord) -> bool:
        record.request_id = _REQUEST_ID.get()
        return True

    collector = _build_filtered_logger(enrich)
    logger = get_logger("app")

    def worker(index: int) -> None:
        token = _REQUEST_ID.set(f"req-{index}")
        try:
            logger.log("INFO", f"message-{index}")
        finally:
            _REQUEST_ID.reset(token)

    threads = [threading.Thread(target=worker, args=(index,)) for index in range(4)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    _wait_for(lambda: len(collector.records) == 4)

    payload = {
        record["message"]: record["metadata"]["key_values"]["request_id"]
        for record in collector.records
    }
    assert payload == {
        "message-0": "req-0",
        "message-1": "req-1",
        "message-2": "req-2",
        "message-3": "req-3",
    }
