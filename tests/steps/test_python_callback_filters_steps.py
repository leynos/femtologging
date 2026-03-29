"""BDD steps for Python callback filter scenarios."""

from __future__ import annotations

import contextvars
import logging
import time
import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    PythonCallbackFilterBuilder,
    get_logger,
    reset_manager,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"
_REQUEST_ID: contextvars.ContextVar[str] = contextvars.ContextVar(
    "bdd_request_id", default=""
)


def enrich_request_id(record: logging.LogRecord) -> bool:
    """Attach the active request ID to the record."""
    record.request_id = _REQUEST_ID.get()
    return True


class RejectAllFilter(logging.Filter):
    """Simple filter object that rejects every record."""

    @typ.override
    def filter(self, record: logging.LogRecord) -> bool:
        """Reject every record passed to the filter."""
        return False


class RecordMetadataPayload(typ.TypedDict):
    """Subset of metadata fields asserted by these BDD scenarios."""

    key_values: dict[str, str]


class CollectedRecordPayload(typ.TypedDict):
    """Structured record subset consumed by the callback-filter steps."""

    metadata: RecordMetadataPayload


class RecordCollector:
    """Collect structured records from ``handle_record``."""

    def __init__(self) -> None:
        """Initialize the in-memory record buffer."""
        self.records: list[CollectedRecordPayload] = []

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback handle method required by registration."""

    def handle_record(self, record: CollectedRecordPayload) -> None:
        """Capture structured record payloads for assertions."""
        self.records.append(record)


def _wait_for(condition: typ.Callable[[], bool], timeout: float = 1.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if condition():
            return
        time.sleep(0.01)
    pytest.fail("condition was not met before timeout")


def _wait_for_quiescence(
    condition: typ.Callable[[], bool],
    *,
    quiet_period: float = 0.05,
    timeout: float = 1.0,
) -> None:
    """Wait until ``condition`` stays true for the whole quiet period."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not condition():
            time.sleep(0.01)
            continue
        quiet_deadline = time.time() + quiet_period
        while time.time() < quiet_deadline:
            if not condition():
                break
            time.sleep(0.01)
        else:
            return
    pytest.fail("condition did not remain stable for the quiet period")


scenarios(str(FEATURES / "python_callback_filters.feature"))


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    reset_manager()
    yield
    reset_manager()


@given("a ConfigBuilder for python callback filters", target_fixture="config_builder")
def config_builder() -> ConfigBuilder:
    return ConfigBuilder()


@when(parsers.parse('I add python callback filter "{fid}" using the enrich callback'))
def add_enrich_filter(config_builder: ConfigBuilder, fid: str) -> None:
    config_builder.with_filter(fid, PythonCallbackFilterBuilder(enrich_request_id))


@when(
    parsers.parse(
        'I add python callback filter "{fid}" using the reject-all filter object'
    )
)
def add_reject_filter(config_builder: ConfigBuilder, fid: str) -> None:
    config_builder.with_filter(fid, RejectAllFilter())


@when(parsers.parse('I add logger "{name}" with python filter "{filter_id}"'))
def add_logger(config_builder: ConfigBuilder, name: str, filter_id: str) -> None:
    config_builder.with_logger(name, LoggerConfigBuilder().with_filters([filter_id]))


@when(parsers.parse('I set the python callback root logger level to "{level}"'))
def set_root(config_builder: ConfigBuilder, level: str) -> None:
    config_builder.with_root_logger(LoggerConfigBuilder().with_level(level))


@then("the python callback filter configuration matches snapshot")
def configuration_matches_snapshot(
    config_builder: ConfigBuilder, snapshot: SnapshotAssertion
) -> None:
    assert config_builder.as_dict() == snapshot


@when("the python callback filter configuration is built")
def build_config(config_builder: ConfigBuilder) -> None:
    config_builder.build_and_init()


@when(
    parsers.parse('I attach a record collector to logger "{name}"'),
    target_fixture="collector",
)
def attach_collector(name: str) -> RecordCollector:
    collector = RecordCollector()
    get_logger(name).add_handler(collector)
    return collector


@when(
    parsers.parse(
        'logger "{name}" emits "{level}" with active request id "{request_id}"'
    )
)
def emit_with_request_id(name: str, level: str, request_id: str) -> None:
    token = _REQUEST_ID.set(request_id)
    try:
        assert get_logger(name).log(level, "hello") is not None
    finally:
        _REQUEST_ID.reset(token)


@then(
    parsers.parse(
        'the collected record metadata contains "{key}" with value "{expected_value}"'
    )
)
def assert_collected_metadata(
    collector: RecordCollector, key: str, expected_value: str
) -> None:
    _wait_for(lambda: len(collector.records) == 1)
    assert collector.records[0]["metadata"]["key_values"][key] == expected_value


@then(
    parsers.parse(
        'logger "{name}" suppresses "{level}" through the python callback filter'
    )
)
def suppresses_record(name: str, level: str, collector: RecordCollector) -> None:
    assert get_logger(name).log(level, "blocked") is None
    _wait_for_quiescence(lambda: collector.records == [])
    assert collector.records == []
