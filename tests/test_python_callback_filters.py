"""Tests for stdlib-compatible Python callback filters."""

from __future__ import annotations

import io
import json
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
from tests.documentation_examples import documented_example, run_example
from tests.python_filter_support import (
    REQUEST_ID,
    PrefixRejectingFilter,
    RecordCollector,
    wait_for,
)
from tests.reentrant_finalizer_support import (
    CLEAR_FILTERS_MARKER,
    CLEAR_FILTERS_SCENARIO,
    REPLACE_CONFIGURATION_MARKER,
    REPLACE_CONFIGURATION_SCENARIO,
    RESET_MANAGER_MARKER,
    RESET_MANAGER_SCENARIO,
    run_reentrant_finalizer_script,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

type FilteredLoggerFactory = cabc.Callable[[object], RecordCollector]

#: Appended to the published example by `run_example`. The example configures
#: and flushes its own logger, so the probe attaches a collector to that same
#: `logger`, emits one more record, and prints the delivered key-values as
#: JSON. Nothing here changes how the shipped text runs. The probe's message
#: differs from the example's, so an assertion on the example's own delivery
#: cannot be satisfied by the probe's record.
PROBE_MESSAGE = "probe after the published example"
RECORD_PROBE = f"""
import json

_collector_records = []


class _Collector:
    def handle(self, logger, level, message):
        pass

    def handle_record(self, record):
        _collector_records.append(dict(record))

    def flush(self):
        return True


logger.add_handler(_Collector())
logger.log("INFO", {PROBE_MESSAGE!r})
logger.flush_handlers()
print(json.dumps(_collector_records[-1]["metadata"]["key_values"]))
"""


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
        """Install the filter on the app logger and return its collector."""
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
        """Stamp the record with the current request ID and accept it."""
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
    capsys: pytest.CaptureFixture[str],
) -> None:
    """Callback exceptions should drop the record without crashing."""

    def boom(record: logging.LogRecord) -> bool:
        """Raise RuntimeError to exercise callback failure handling."""
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
    diagnostics = capsys.readouterr().err
    assert "Python filter function failed: RuntimeError: boom: hello" in diagnostics, (
        "callback failure diagnostics must identify the filter and original exception"
    )


def test_python_callback_filters_preserve_exception_fields(
    filtered_logger: FilteredLoggerFactory,
) -> None:
    """Callback records should include emitted ``exc_info`` and ``stack_info``."""
    seen: dict[str, object] = {}

    def require_exception_state(record: logging.LogRecord) -> bool:
        """Record exc_info and stack_info, accepting only when both are set."""
        seen["exc_info"] = record.exc_info
        seen["stack_info"] = record.stack_info
        return record.exc_info is not None and bool(record.stack_info)

    def raise_boom() -> None:
        """Raise a ValueError so the caller captures exception state."""
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
        """Stamp the record with a request ID and accept it."""
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
        """Copy the current request ID onto the record and accept it."""
        record.request_id = REQUEST_ID.get()
        return True

    collector = filtered_logger(enrich)
    logger = get_logger("app")

    def worker(index: int) -> None:
        """Log one message with a thread-local request ID."""
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


def test_documented_callback_enrichment_example_enriches_records(
    tmp_path: Path,
) -> None:
    """The published users' guide example configures an enriching filter.

    The example runs exactly as published in a fresh interpreter, so a guide
    that drifts back to an API the library does not support fails here instead
    of misleading a reader. The appended probe only reports what the example
    itself cannot: the key-values its filter put on a delivered record.
    """
    example = documented_example("callback-enrichment-accepted")
    result = run_example(example, tmp_path, probe=RECORD_PROBE)

    assert result.returncode == 0, f"the published example failed:\n{result.stderr}"
    # The probe emits a different message, so finding this text proves the
    # example's own `logger.log(...)` reached a handler and was delivered.
    assert "request accepted" in result.stderr, (
        f"the published example must deliver its own record, got {result.stderr!r}"
    )
    assert result.stdout.strip(), "the probe must report the delivered record"
    key_values = json.loads(result.stdout.strip().splitlines()[-1])
    assert key_values == {
        "request_id": "req-123",
        "user_id": "42",
        "cache_hit": "False",
    }, f"the documented filter must enrich the record, got {key_values!r}"
@pytest.mark.timeout(20)
def test_clear_filters_runs_a_callback_finalizer_after_releasing_the_lock() -> None:
    """A callback filter finalizer may log to the same logger.

    The registry releases its reference first, leaving the logger's filter
    collection as the last owner, so ``clear_filters`` is what triggers the
    finalizer's re-entrant ``info`` call.
    """
    run_reentrant_finalizer_script(CLEAR_FILTERS_SCENARIO, CLEAR_FILTERS_MARKER)


@pytest.mark.timeout(20)
def test_configuration_replacement_runs_a_callback_finalizer() -> None:
    """Replacing a configuration destroys displaced callbacks without locks.

    The callback leaves both the registry and the live logger in one commit;
    its finalizer re-enters ``get_logger`` before printing the marker.
    """
    run_reentrant_finalizer_script(
        REPLACE_CONFIGURATION_SCENARIO,
        REPLACE_CONFIGURATION_MARKER,
    )


@pytest.mark.timeout(20)
def test_reset_manager_runs_a_callback_finalizer_after_releasing_the_manager() -> None:
    """``reset_manager`` drops runtime state after releasing ``MANAGER``.

    The live logger's reference is cleared first, so the runtime registry is
    the callback's last owner and the finalizer's ``get_logger`` call would
    deadlock on ``MANAGER`` if the snapshot were dropped under the guard.
    """
    run_reentrant_finalizer_script(RESET_MANAGER_SCENARIO, RESET_MANAGER_MARKER)
