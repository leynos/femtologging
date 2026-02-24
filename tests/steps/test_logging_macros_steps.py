"""BDD steps for module-level logging convenience function scenarios."""

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

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "logging_macros.feature"))


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture
def log_result() -> dict[str, object]:
    """Mutable container for passing log results between steps."""
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
def call_convenience_func(func: str, message: str) -> dict[str, object]:
    """Call a module-level convenience function and capture the result."""
    fn = _FUNC_MAP[func]
    return {"value": fn(message)}


@when(
    parsers.parse('I call {func} with message "{message}" and name "{name}"'),
    target_fixture="log_result",
)
def call_convenience_func_with_name(
    func: str, message: str, name: str
) -> dict[str, object]:
    """Call a module-level convenience function targeting a named logger."""
    fn = _FUNC_MAP[func]
    return {"value": fn(message, name=name)}


# ---------------------------------------------------------------------------
# Then steps
# ---------------------------------------------------------------------------


@then("the result is not None")
def result_is_not_none(log_result: dict[str, object]) -> None:
    """Assert that the log result is not None (record was emitted)."""
    assert log_result["value"] is not None, (
        f"Expected non-None result, got {log_result['value']!r}"
    )


@then("the result is None")
def result_is_none(log_result: dict[str, object]) -> None:
    """Assert that the log result is None (record was suppressed)."""
    assert log_result["value"] is None, (
        f"Expected None result, got {log_result['value']!r}"
    )


@then(parsers.parse('the result contains "{text}"'))
def result_contains(log_result: dict[str, object], text: str) -> None:
    """Assert that the formatted log output contains the specified text."""
    value = log_result["value"]
    assert value is not None, "Result is None, cannot check contents"
    assert text in str(value), f"Expected '{text}' in '{value}'"


@then("the info result matches snapshot")
def info_result_matches_snapshot(
    log_result: dict[str, object], snapshot: SnapshotAssertion
) -> None:
    """Assert that the info result matches the stored snapshot.

    Source location details (file path and line number) are normalised
    to stable placeholders before comparison so that the snapshot is
    reproducible regardless of the test runner's working directory.
    """
    value = log_result["value"]
    assert value is not None, "Result is None, cannot snapshot"
    normalised = _normalise_source_location(str(value))
    assert normalised == snapshot, (
        f"Normalised output did not match snapshot: {normalised!r}"
    )


@then(parsers.parse('the result format is "{expected}"'))
def result_format_is(log_result: dict[str, object], expected: str) -> None:
    """Assert the formatted output matches the expected string exactly."""
    value = log_result["value"]
    assert value is not None, "Result is None, cannot check format"
    assert str(value) == expected, f"Expected '{expected}', got '{value}'"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _normalise_source_location(output: str) -> str:
    """Replace file paths and line numbers with stable placeholders."""
    # Normalise file paths (e.g., /foo/bar/baz.py -> <file>)
    result = re.sub(r"[^\s:]+\.py", "<file>", output)
    # Normalise line numbers (e.g., :42 -> :<N>)
    return re.sub(r":\d+", ":<N>", result)
