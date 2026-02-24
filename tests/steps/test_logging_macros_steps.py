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
import typing as typ
from pathlib import Path
from types import MappingProxyType

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    debug,
    error,
    info,
    warn,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy import SnapshotAssertion


class LogResultPayload(typ.TypedDict):
    """Payload dict shuttled between ``@when`` and ``@then`` steps."""

    value: str | None


FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "logging_macros.feature"))


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def log_result() -> LogResultPayload:
    """Provide the initial log-result payload.

    Replaced by ``@when`` steps via ``target_fixture``.
    """
    return {"value": None}


# ---------------------------------------------------------------------------
# Given steps
# ---------------------------------------------------------------------------


@given(
    parsers.parse('a logger named "{name}" with level "{level}"'),
    target_fixture="named_logger_config",
)
def given_named_logger(name: str, level: str) -> str:
    """Configure a named logger with the specified level."""
    builder = ConfigBuilder()
    root = LoggerConfigBuilder().with_level("DEBUG")
    builder.with_root_logger(root)
    child = LoggerConfigBuilder().with_level(level)
    builder.with_logger(name, child)
    builder.build_and_init()
    return name


# ---------------------------------------------------------------------------
# When steps
# ---------------------------------------------------------------------------

_FUNC_MAP: cabc.Mapping[str, cabc.Callable[..., str | None]] = MappingProxyType({
    "info": info,
    "debug": debug,
    "warn": warn,
    "error": error,
})


@when(
    parsers.parse('I call {func} with message "{message}"'),
    target_fixture="log_result",
)
def call_convenience_func(func: str, message: str) -> LogResultPayload:
    """Call a module-level convenience function and capture the result."""
    fn = _FUNC_MAP[func]
    return {"value": fn(message)}


@when(
    parsers.parse('I call {func} with message "{message}" and name "{name}"'),
    target_fixture="log_result",
)
def call_convenience_func_with_name(
    func: str, message: str, name: str
) -> LogResultPayload:
    """Call a module-level convenience function targeting a named logger."""
    fn = _FUNC_MAP[func]
    return {"value": fn(message, name=name)}


# ---------------------------------------------------------------------------
# Then steps
# ---------------------------------------------------------------------------


@then("the result is not None")
def result_is_not_none(log_result: LogResultPayload) -> None:
    """Assert that the log result is not None (record was emitted)."""
    assert log_result["value"] is not None, (
        f"Expected non-None result, got {log_result['value']!r}"
    )


@then("the result is None")
def result_is_none(log_result: LogResultPayload) -> None:
    """Assert that the log result is None (record was suppressed)."""
    assert log_result["value"] is None, (
        f"Expected None result, got {log_result['value']!r}"
    )


@then(parsers.parse('the result contains "{text}"'))
def result_contains(log_result: LogResultPayload, text: str) -> None:
    """Assert that the formatted log output contains the specified text."""
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
    """
    value = log_result["value"]
    assert value is not None, "Result is None, cannot snapshot"
    normalized = _normalize_source_location(str(value))
    assert normalized == snapshot, (
        f"Normalized output did not match snapshot: {normalized!r}"
    )


@then(parsers.parse('the result format is "{expected}"'))
def result_format_is(log_result: LogResultPayload, expected: str) -> None:
    """Assert the formatted output matches the expected string exactly."""
    value = log_result["value"]
    assert value is not None, "Result is None, cannot check format"
    assert str(value) == expected, f"Expected '{expected}', got '{value}'"


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
