"""pytest-bdd step implementations for logging macro / convenience function scenarios.

Scenarios are defined in ``tests/features/logging_macros.feature`` and exercise
the module-level convenience functions (``info``, ``debug``, ``warn``,
``error``) exposed by femtologging.

The ``log_result`` fixture is a mutable ``dict`` (``{"value": ...}``) that
shuttles data between steps.  ``@when`` steps overwrite it via
``target_fixture="log_result"`` so the return value becomes the new fixture
instance, and ``@then`` steps receive the same dict to run assertions against
``log_result["value"]``.
"""

from __future__ import annotations

import re
import time
import typing as typ
from contextlib import contextmanager
from pathlib import Path
from types import MappingProxyType

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    debug,
    error,
    get_logger,
    info,
    log_context,
    warn,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy import SnapshotAssertion


class LogResultPayload(typ.TypedDict):
    """Payload dict shuttled between ``@when`` and ``@then`` steps."""

    value: str | None


class __MetadataPayload(typ.TypedDict):
    """Structured metadata captured from ``handle_record`` callbacks."""

    value: dict[str, str]


class __ErrorPayload(typ.TypedDict):
    """Error payload captured for unhappy-path assertions."""

    value: str | None


class _Record_MetadataPayload(typ.TypedDict):
    """Subset of record metadata used in these behavioural assertions."""

    key_values: dict[str, object]


class _CapturedRecordPayload(typ.TypedDict):
    """Subset of captured record payloads consumed by helper assertions."""

    metadata: _Record_MetadataPayload


class __FlushableLogger(typ.Protocol):
    """Structural type for logger objects that expose ``flush_handlers``."""

    def flush_handlers(self) -> bool:
        """Flush pending records and return whether the flush succeeded."""

    def clear_handlers(self) -> None:
        """Remove all handlers before test-scoped capture."""

    def add_handler(self, handler: object) -> None:
        """Attach a handler for the current capture scope."""

    def remove_handler(self, handler: object) -> None:
        """Detach a handler when capture scope exits."""


FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "logging_macros.feature"))


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def log_result() -> LogResultPayload:
    """Provide the initial log-result payload.

    Replaced by ``@when`` steps via ``target_fixture``.

    Returns
    -------
    LogResultPayload
        Dict with ``"value"`` set to ``None``.

    """
    return {"value": None}


@pytest.fixture
def metadata_payload() -> _MetadataPayload:
    """Provide structured metadata storage for context scenarios."""
    return {"value": {}}


@pytest.fixture
def context_error() -> _ErrorPayload:
    """Provide context error storage for unhappy-path scenarios."""
    return {"value": None}


# ---------------------------------------------------------------------------
# Given steps
# ---------------------------------------------------------------------------


@given(
    parsers.parse('a logger named "{name}" with level "{level}"'),
    target_fixture="named_logger_config",
)
def given_named_logger(name: str, level: str) -> str:
    """Configure a named logger with the specified level.

    Parameters
    ----------
    name : str
        Logger name to register.
    level : str
        Logging threshold (e.g., ``"DEBUG"``, ``"INFO"``).

    Returns
    -------
    str
        The logger name, exposed as the ``named_logger_config``
        fixture.

    """
    builder = ConfigBuilder()
    root = LoggerConfigBuilder().with_level("DEBUG")
    builder.with_root_logger(root)
    child = LoggerConfigBuilder().with_level(level)
    builder.with_logger(name, child)
    builder.build_and_init()
    return name


@given(parsers.parse('a record-collecting logger named "{name}" with level "{level}"'))
def given_record_collecting_logger(name: str, level: str) -> None:
    """Configure a named logger used by context metadata scenarios."""
    builder = ConfigBuilder()
    builder.with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
    builder.with_logger(name, LoggerConfigBuilder().with_level(level))
    builder.build_and_init()


# ---------------------------------------------------------------------------
# When steps
# ---------------------------------------------------------------------------

_FUNC_MAP: cabc.Mapping[str, cabc.Callable[..., str | None]] = MappingProxyType({
    "info": info,
    "debug": debug,
    "warn": warn,
    "error": error,
})

_CALL_WITH_CONTEXT_PATTERN = (
    r'I call (?P<func>\w+) with message "(?P<message>[^"]+)" and name '
    r'"(?P<name>[^"]+)" inside context (?P<context>.+)'
)
_CALL_WITH_NAME_PATTERN = (
    r'^I call (?P<func>\w+) with message "(?P<message>[^"]+)" and name '
    r'"(?P<name>[^"]+)"$'
)
_CALL_WITH_MESSAGE_PATTERN = r'^I call (?P<func>\w+) with message "(?P<message>[^"]+)"$'
_CALL_WITH_NESTED_CONTEXT_PATTERN = (
    r'I call (?P<func>\w+) with message "(?P<message>[^"]+)" and name '
    r'"(?P<name>[^"]+)" inside nested context (?P<contexts>.+)'
)
_EXPECT_KEY_VALUES_PATTERN = (
    r"the latest record metadata key_values contain (?P<pairs>.+)"
)


class _RecordCollector:
    """Collect full records passed to ``handle_record`` callbacks."""

    def __init__(self) -> None:
        """Initialize collector state for one scenario."""
        self.records: list[_CapturedRecordPayload] = []

    def handle(self, logger: str, level: str, message: str) -> None:
        """Accept classic handler calls for compatibility with logger handlers."""
        _ = (self.records, logger, level, message)

    def handle_record(self, record: _CapturedRecordPayload) -> None:
        """Capture full record payloads for metadata assertions."""
        self.records.append(record)

    def flush(self) -> bool:
        """Report successful flush to satisfy ``flush_handlers`` checks."""
        _ = self.records
        return True


@when(
    parsers.re(_CALL_WITH_MESSAGE_PATTERN),
    target_fixture="log_result",
)
def call_convenience_func(func: str, message: str) -> LogResultPayload:
    """Call a module-level convenience function and capture the result.

    Parameters
    ----------
    func : str
        Key into ``_FUNC_MAP`` (e.g., ``"info"``, ``"debug"``).
    message : str
        Log message to emit.

    Returns
    -------
    LogResultPayload
        Dict with ``"value"`` set to the function's return value.

    """
    fn = _FUNC_MAP[func]
    return {"value": fn(message)}


@when(
    parsers.re(_CALL_WITH_NAME_PATTERN),
    target_fixture="log_result",
)
def call_convenience_func_with_name(
    func: str, message: str, name: str
) -> LogResultPayload:
    """Call a module-level convenience function targeting a named logger.

    Parameters
    ----------
    func : str
        Key into ``_FUNC_MAP`` (e.g., ``"error"``).
    message : str
        Log message to emit.
    name : str
        Logger name passed as the ``name`` keyword argument.

    Returns
    -------
    LogResultPayload
        Dict with ``"value"`` set to the function's return value.

    """
    fn = _FUNC_MAP[func]
    return {"value": fn(message, name=name)}


@when(
    parsers.re(_CALL_WITH_CONTEXT_PATTERN),
    target_fixture="metadata_payload",
)
def call_with_context_and_capture_metadata(
    func: str,
    message: str,
    name: str,
    context: str,
) -> _MetadataPayload:
    """Emit a log call inside ``log_context`` and capture key-values."""
    context_map = _parse_pairs(context)
    fn = _FUNC_MAP[func]
    latest = _capture_latest_key_values(
        logger_name=name,
        fn=fn,
        message=message,
        context=context_map,
    )
    return {"value": latest}


@when(
    parsers.re(_CALL_WITH_NESTED_CONTEXT_PATTERN),
    target_fixture="metadata_payload",
)
def call_with_nested_context_and_capture_metadata(
    func: str,
    message: str,
    name: str,
    contexts: str,
) -> _MetadataPayload:
    """Emit one log call with nested contexts and capture key-values."""
    outer, inner = _split_nested_contexts(contexts)
    outer_map = _parse_pairs(outer)
    inner_map = _parse_pairs(inner)
    logger = get_logger(name)
    fn = _FUNC_MAP[func]
    with _capture_records(logger) as collector:
        with log_context(**outer_map), log_context(**inner_map):
            fn(message, name=name)
        latest = _wait_for_latest_key_values(logger, collector)
    return {"value": {str(k): str(v) for k, v in latest.items()}}


@when("I push log context with an invalid nested value", target_fixture="context_error")
def push_invalid_context_value() -> _ErrorPayload:
    """Capture error text when pushing unsupported context value types."""
    message: str | None = None
    try:
        with log_context(bad={"nested": "dict"}):
            pass
    except TypeError as exc:
        message = str(exc)
    return {"value": message}


# ---------------------------------------------------------------------------
# Then steps
# ---------------------------------------------------------------------------


@then("the result is not None")
def result_is_not_none(log_result: LogResultPayload) -> None:
    """Assert that the log result is not None (record was emitted).

    Parameters
    ----------
    log_result : LogResultPayload
        Payload produced by a preceding ``@when`` step.

    """
    assert log_result["value"] is not None, (
        f"Expected non-None result, got {log_result['value']!r}"
    )


@then("the result is None")
def result_is_none(log_result: LogResultPayload) -> None:
    """Assert that the log result is None (record was suppressed).

    Parameters
    ----------
    log_result : LogResultPayload
        Payload produced by a preceding ``@when`` step.

    """
    assert log_result["value"] is None, (
        f"Expected None result, got {log_result['value']!r}"
    )


@then(parsers.parse('the result contains "{text}"'))
def result_contains(log_result: LogResultPayload, text: str) -> None:
    """Assert that the formatted log output contains the specified text.

    Parameters
    ----------
    log_result : LogResultPayload
        Payload produced by a preceding ``@when`` step.
    text : str
        Substring expected in the formatted log output.

    """
    value = log_result["value"]
    assert value is not None, "Result is None, cannot check contents"
    assert text in str(value), f"Expected '{text}' in '{value}'"


@then("the info result matches snapshot")
def info_result_matches_snapshot(
    log_result: LogResultPayload, snapshot: SnapshotAssertion
) -> None:
    """Assert that the info result matches the stored snapshot.

    Source location details (file path and line number) are normalized
    to stable placeholders before comparison so that the snapshot is
    reproducible regardless of the test runner's working directory.

    Parameters
    ----------
    log_result : LogResultPayload
        Payload produced by a preceding ``@when`` step.
    snapshot : SnapshotAssertion
        Syrupy snapshot to compare against.

    """
    value = log_result["value"]
    assert value is not None, "Result is None, cannot snapshot"
    normalized = _normalize_source_location(str(value))
    assert normalized == snapshot, (
        f"Normalized output did not match snapshot: {normalized!r}"
    )


@then(parsers.parse('the result format is "{expected}"'))
def result_format_is(log_result: LogResultPayload, expected: str) -> None:
    """Assert the formatted output matches the expected string exactly.

    Parameters
    ----------
    log_result : LogResultPayload
        Payload produced by a preceding ``@when`` step.
    expected : str
        Exact string the formatted output must equal.

    """
    value = log_result["value"]
    assert value is not None, "Result is None, cannot check format"
    assert str(value) == expected, f"Expected '{expected}', got '{value}'"


@then(parsers.re(_EXPECT_KEY_VALUES_PATTERN))
def key_values_contain_expected_pairs(
    metadata_payload: _MetadataPayload, pairs: str
) -> None:
    """Assert captured metadata includes expected key-values."""
    expected = _parse_pairs(pairs)
    key_values = metadata_payload["value"]
    for key, value in expected.items():
        assert key_values.get(key) == value, f"missing {key}={value}"


@then(parsers.parse('a context error is raised containing "{text}"'))
def context_error_contains(context_error: _ErrorPayload, text: str) -> None:
    """Assert invalid context operations report deterministic errors."""
    value = context_error["value"]
    assert value is not None, "expected context error, got none"
    assert text in value, f"expected {text!r} in {value!r}"


@then("the latest record metadata key_values match snapshot")
def key_values_match_snapshot(
    metadata_payload: _MetadataPayload, snapshot: SnapshotAssertion
) -> None:
    """Assert metadata key-values for context scenarios match the snapshot."""
    assert metadata_payload["value"] == snapshot


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _normalize_source_location(output: str) -> str:
    """Replace file paths and line numbers with stable placeholders."""
    # Normalize file paths (e.g., /foo/bar/baz.py or C:\foo\bar.py -> <file>)
    # Optional drive letter, forward/back-slash separators, lookahead for :line
    result = re.sub(r"(?:[A-Za-z]:)?[^\s:]+\.py(?=:\d+)", "<file>", output)
    # Normalize line numbers (e.g., :42 -> :<N>)
    return re.sub(r":\d+", ":<N>", result)


def _parse_pairs(text: str) -> dict[str, str]:
    """Parse ``"key"="value"`` pairs joined by ``and``."""
    pattern = re.compile(r'"([^"]+)"="([^"]*)"')
    pairs = dict(pattern.findall(text))
    assert pairs, f"expected at least one key-value pair in {text!r}"
    return pairs


def _split_nested_contexts(text: str) -> tuple[str, str]:
    """Split ``outer then inner`` context expressions."""
    outer, sep, inner = text.partition(" then ")
    assert sep, f"expected nested context separator in {text!r}"
    return outer, inner


def _capture_latest_key_values(
    *,
    logger_name: str,
    fn: cabc.Callable[..., str | None],
    message: str,
    context: dict[str, str],
) -> dict[str, str]:
    """Emit a record and return captured metadata key-values."""
    logger = get_logger(logger_name)
    with _capture_records(logger) as collector:
        with log_context(**context):
            fn(message, name=logger_name)
        latest = _wait_for_latest_key_values(logger, collector)
    return {str(k): str(v) for k, v in latest.items()}


@contextmanager
def _capture_records(logger: _FlushableLogger) -> typ.Iterator[_RecordCollector]:
    """Attach a short-lived collector after draining pending records."""
    logger.clear_handlers()
    flushed = logger.flush_handlers()
    assert flushed, "flush_handlers() failed before attaching context collector"
    collector = _RecordCollector()
    logger.add_handler(collector)
    try:
        yield collector
    finally:
        logger.remove_handler(collector)


def _wait_for_latest_key_values(
    logger: _FlushableLogger,
    collector: _RecordCollector,
    *,
    attempts: int = 20,
    interval_s: float = 0.01,
) -> dict[str, object]:
    """Wait for a captured record and return its latest key-values payload."""
    for _ in range(attempts):
        if collector.records:
            break
        time.sleep(interval_s)
        flushed = logger.flush_handlers()
        assert flushed, "flush_handlers() failed while waiting for captured records"
    assert collector.records, "expected at least one captured record"
    return collector.records[-1]["metadata"]["key_values"]
