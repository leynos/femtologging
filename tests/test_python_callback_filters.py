"""Tests for stdlib-compatible Python callback filters."""

from __future__ import annotations

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
    get_logger,
)
from tests.python_filter_support import (
    REQUEST_ID,
    PrefixRejectingFilter,
    RecordCollector,
    wait_for,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

type FilteredLoggerFactory = cabc.Callable[[object], RecordCollector]


@pytest.fixture(autouse=True)
def _reset_request_id() -> cabc.Iterator[None]:
    """Clear the correlation contextvar around each test.

    The global logging manager is reset by the autouse fixture in
    ``tests/conftest.py``; only the contextvar needs handling here.
    """
    REQUEST_ID.set("")
    yield
    REQUEST_ID.set("")


@pytest.fixture
def filtered_logger() -> FilteredLoggerFactory:
    """Return a factory that installs ``app`` behind a single named filter.

    Returns
    -------
    FilteredLoggerFactory
        Callable taking the filter object and returning the collector
        attached to the configured ``app`` logger.
    """

    def build(filter_obj: object) -> RecordCollector:
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

    return build


def test_python_callback_filter_builder_persists_contextvar_enrichment(
    filtered_logger: FilteredLoggerFactory,
) -> None:
    """Accepted callback filters should persist new record attributes."""

    def enrich(record: logging.LogRecord) -> bool:
        record.correlation_id = REQUEST_ID.get()
        return True

    collector = filtered_logger(PythonCallbackFilterBuilder(enrich))
    REQUEST_ID.set("req-123")

    logger = get_logger("app")
    assert logger.log("INFO", "hello") is not None, (
        "an accepting callback filter must let the record through"
    )
    wait_for(lambda: len(collector.records) == 1, "the record reaches the collector")

    record = collector.records[0]
    assert record["metadata"]["key_values"]["correlation_id"] == "req-123", (
        "the attribute set by the callback must survive into record metadata, "
        f"got {record['metadata']['key_values']!r}"
    )


def test_python_filter_object_rejects_records(
    filtered_logger: FilteredLoggerFactory,
) -> None:
    """Objects exposing ``filter(record)`` should participate in filtering."""
    filtered_logger(PrefixRejectingFilter("svc"))

    logger = get_logger("app")
    assert logger.log("INFO", "blocked") is None, (
        "a filter object rejecting the logger name must suppress the record"
    )


def test_python_callback_filter_builder_rejects_invalid_targets() -> None:
    """Invalid callback targets should raise a consistent ``TypeError``."""
    with pytest.raises(
        TypeError,
        match=(
            "python callback filter must be callable or expose a callable "
            "'filter' method"
        ),
    ):
        PythonCallbackFilterBuilder(object())


def test_python_callback_filter_exceptions_drop_records(
    filtered_logger: FilteredLoggerFactory,
) -> None:
    """Callback exceptions should drop the record without crashing."""

    def boom(record: logging.LogRecord) -> bool:
        message = f"boom: {record.msg}"
        raise RuntimeError(message)

    collector = filtered_logger(PythonCallbackFilterBuilder(boom))

    logger = get_logger("app")
    assert logger.log("INFO", "hello") is None, (
        "a callback filter that raises must drop the record rather than emit it"
    )
    time.sleep(0.05)
    assert not collector.records, (
        "a raising callback filter must not deliver anything to handlers, "
        f"got {collector.records!r}"
    )


def test_python_callback_filters_preserve_exception_fields(
    filtered_logger: FilteredLoggerFactory,
) -> None:
    """Callback records should include emitted ``exc_info`` and ``stack_info``."""
    seen: dict[str, object] = {}

    def require_exception_state(record: logging.LogRecord) -> bool:
        seen["exc_info"] = record.exc_info
        seen["stack_info"] = record.stack_info
        return record.exc_info is not None and bool(record.stack_info)

    def raise_boom() -> None:
        message = "boom"
        raise ValueError(message)

    collector = filtered_logger(PythonCallbackFilterBuilder(require_exception_state))
    logger = get_logger("app")

    try:
        raise_boom()
    except ValueError:
        emitted = logger.log("ERROR", "caught", exc_info=True, stack_info=True)
    assert emitted is not None, (
        "the filter accepts only when exc_info and stack_info are populated, "
        "so the record must be emitted"
    )

    wait_for(lambda: len(collector.records) == 1, "the record reaches the collector")
    exc_info = typ.cast("dict[str, object]", seen["exc_info"])
    stack_info = typ.cast("dict[str, object]", seen["stack_info"])
    assert exc_info["type_name"] == "ValueError", (
        f"the filter must observe the raised exception type, got {exc_info!r}"
    )
    assert stack_info["frames"], (
        f"the filter must observe a non-empty stack payload, got {stack_info!r}"
    )


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

    assert get_logger("app").log("INFO", "hello") is not None, (
        "the enriching filter must accept the record"
    )
    wait_for(
        lambda: stream.getvalue().strip() == "abc-123 hello",
        "the adapter formats the enriched record",
    )
    assert stream.getvalue().strip() == "abc-123 hello", (
        "the stdlib formatter must see the injected request_id, got "
        f"{stream.getvalue()!r}"
    )


def test_contextvar_enrichment_is_thread_local(
    filtered_logger: FilteredLoggerFactory,
) -> None:
    """Concurrent callback filters should preserve per-thread contextvars."""

    def enrich(record: logging.LogRecord) -> bool:
        record.request_id = REQUEST_ID.get()
        return True

    collector = filtered_logger(enrich)
    logger = get_logger("app")

    def worker(index: int) -> None:
        token = REQUEST_ID.set(f"req-{index}")
        try:
            logger.log("INFO", f"message-{index}")
        finally:
            REQUEST_ID.reset(token)

    threads = [threading.Thread(target=worker, args=(index,)) for index in range(4)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()

    wait_for(
        lambda: len(collector.records) == len(threads),
        "every worker record reaches the collector",
    )

    payload = {
        record["message"]: record["metadata"]["key_values"]["request_id"]
        for record in collector.records
    }
    expected = {f"message-{index}": f"req-{index}" for index in range(len(threads))}
    assert payload == expected, (
        "each thread's contextvar must be captured against its own record, "
        f"got {payload!r}"
    )
