"""pytest-bdd step implementations for logging macro / convenience function scenarios.

Scenarios are defined in ``tests/features/logging_macros.feature`` and exercise
the module-level convenience functions (``info``, ``debug``, ``warn``,
``error``) exposed by femtologging.

The ``log_result`` fixture is a mutable ``dict`` (``{"value": ...}``) that
shuttles data between steps.  ``@when`` steps overwrite it via
``target_fixture="log_result"`` so the return value becomes the new fixture
instance, and ``@then`` steps receive the same dict to run assertions against
``log_result["value"]``.

Payload types, the record-collecting test double, and the parsing helpers
these steps rely on live in ``tests/steps/logging_macros_support.py``.
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import ConfigBuilder, LoggerConfigBuilder, get_logger, log_context
from tests.steps.logging_macros_support import (
    CALL_WITH_CONTEXT_PATTERN,
    CALL_WITH_MESSAGE_PATTERN,
    CALL_WITH_NAME_PATTERN,
    CALL_WITH_NESTED_CONTEXT_PATTERN,
    EXPECT_KEY_VALUES_PATTERN,
    FUNC_MAP,
    ErrorPayload,
    LogResultPayload,
    MetadataPayload,
    capture_records,
    latest_key_values,
    normalize_source_location,
    parse_pairs,
    split_nested_contexts,
    wait_for_record,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy import SnapshotAssertion

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
def metadata_payload() -> MetadataPayload:
    """Provide structured metadata storage for context scenarios."""
    return {"value": {}}


@pytest.fixture
def context_error() -> ErrorPayload:
    """Provide context error storage for unhappy-path scenarios."""
    return {"value": None}


# ---------------------------------------------------------------------------
# Given steps
# ---------------------------------------------------------------------------


def _init_logger(name: str, level: str) -> None:
    """Initialize the global config with a root logger and one named child."""
    builder = ConfigBuilder()
    builder.with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
    builder.with_logger(name, LoggerConfigBuilder().with_level(level))
    builder.build_and_init()


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
    _init_logger(name, level)
    return name


@given(parsers.parse('a record-collecting logger named "{name}" with level "{level}"'))
def given_record_collecting_logger(name: str, level: str) -> None:
    """Configure a named logger used by context metadata scenarios."""
    _init_logger(name, level)


# ---------------------------------------------------------------------------
# When steps
# ---------------------------------------------------------------------------


@when(
    parsers.re(CALL_WITH_MESSAGE_PATTERN),
    target_fixture="log_result",
)
def call_convenience_func(func: str, message: str) -> LogResultPayload:
    """Call a module-level convenience function and capture the result.

    Parameters
    ----------
    func : str
        Key into ``FUNC_MAP`` (e.g., ``"info"``, ``"debug"``).
    message : str
        Log message to emit.

    Returns
    -------
    LogResultPayload
        Dict with ``"value"`` set to the function's return value.

    """
    return {"value": FUNC_MAP[func](message)}


@when(
    parsers.re(CALL_WITH_NAME_PATTERN),
    target_fixture="log_result",
)
def call_convenience_func_with_name(
    func: str, message: str, name: str
) -> LogResultPayload:
    """Call a module-level convenience function targeting a named logger.

    Parameters
    ----------
    func : str
        Key into ``FUNC_MAP`` (e.g., ``"error"``).
    message : str
        Log message to emit.
    name : str
        Logger name passed as the ``name`` keyword argument.

    Returns
    -------
    LogResultPayload
        Dict with ``"value"`` set to the function's return value.

    """
    return {"value": FUNC_MAP[func](message, name=name)}


def _capture_key_values(
    logger_name: str,
    emit: cabc.Callable[[], None],
) -> dict[str, str]:
    """Run *emit* against a temporary collector and return captured key-values."""
    logger = get_logger(logger_name)
    with capture_records(logger) as collector:
        emit()
        wait_for_record(collector)
        latest = latest_key_values(collector)
    return {str(key): str(value) for key, value in latest.items()}


@when(
    parsers.re(CALL_WITH_CONTEXT_PATTERN),
    target_fixture="metadata_payload",
)
def call_with_context_and_capture_metadata(
    func: str,
    message: str,
    name: str,
    context: str,
) -> MetadataPayload:
    """Emit a log call inside ``log_context`` and capture key-values."""
    context_map = parse_pairs(context)

    def emit() -> None:
        with log_context(**context_map):
            FUNC_MAP[func](message, name=name)

    return {"value": _capture_key_values(name, emit)}


@when(
    parsers.re(CALL_WITH_NESTED_CONTEXT_PATTERN),
    target_fixture="metadata_payload",
)
def call_with_nested_context_and_capture_metadata(
    func: str,
    message: str,
    name: str,
    contexts: str,
) -> MetadataPayload:
    """Emit one log call with nested contexts and capture key-values."""
    outer, inner = split_nested_contexts(contexts)
    outer_map = parse_pairs(outer)
    inner_map = parse_pairs(inner)

    def emit() -> None:
        with log_context(**outer_map), log_context(**inner_map):
            FUNC_MAP[func](message, name=name)

    return {"value": _capture_key_values(name, emit)}


@when("I push log context with an invalid nested value", target_fixture="context_error")
def push_invalid_context_value() -> ErrorPayload:
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
    normalized = normalize_source_location(str(value))
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


@then(parsers.re(EXPECT_KEY_VALUES_PATTERN))
def key_values_contain_expected_pairs(
    metadata_payload: MetadataPayload, pairs: str
) -> None:
    """Assert captured metadata includes expected key-values."""
    expected = parse_pairs(pairs)
    key_values = metadata_payload["value"]
    for key, value in expected.items():
        assert key_values.get(key) == value, f"missing {key}={value}"


@then(parsers.parse('a context error is raised containing "{text}"'))
def context_error_contains(context_error: ErrorPayload, text: str) -> None:
    """Assert invalid context operations report deterministic errors."""
    value = context_error["value"]
    assert value is not None, "expected context error, got none"
    assert text in value, f"expected {text!r} in {value!r}"


@then("the latest record metadata key_values match snapshot")
def key_values_match_snapshot(
    metadata_payload: MetadataPayload, snapshot: SnapshotAssertion
) -> None:
    """Assert metadata key-values for context scenarios match the snapshot."""
    assert metadata_payload["value"] == snapshot, (
        f"metadata payload key_values {metadata_payload['value']!r} "
        f"did not match snapshot"
    )
