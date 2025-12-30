"""BDD steps for stack info logging scenarios."""

from __future__ import annotations

import re
import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import FemtoLogger

if typ.TYPE_CHECKING:
    from syrupy import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "stack_info.feature"))


@pytest.fixture
def logger_fixture() -> dict[str, typ.Any]:
    """Storage for the logger and related state."""
    return {"logger": None, "output": None}


@pytest.fixture
def exception_state() -> dict[str, typ.Any]:
    """Storage for exception state."""
    return {"active": False, "type": None, "message": None}


@given(parsers.parse('a logger named "{name}"'))
def create_logger(logger_fixture: dict[str, typ.Any], name: str) -> None:
    logger_fixture["logger"] = FemtoLogger(name)


@given(parsers.parse('an active ValueError exception with message "{message}"'))
def set_active_value_error(exception_state: dict[str, typ.Any], message: str) -> None:
    exception_state["active"] = True
    exception_state["type"] = ValueError
    exception_state["message"] = message


@when(parsers.parse('I log at {level} with message "{message}" and stack_info=True'))
def log_with_stack_info(
    logger_fixture: dict[str, typ.Any],
    level: str,
    message: str,
) -> None:
    logger = logger_fixture["logger"]
    logger_fixture["output"] = logger.log(level, message, stack_info=True)


@when(parsers.parse('I log at {level} with message "{message}"'))
def log_without_extras(
    logger_fixture: dict[str, typ.Any],
    level: str,
    message: str,
) -> None:
    logger = logger_fixture["logger"]
    logger_fixture["output"] = logger.log(level, message)


def _raise_exception(exc_type: type[BaseException], message: str) -> None:
    """Raise an exception of the given type with the given message."""
    raise exc_type(message)


@when(
    parsers.parse(
        'I log at {level} with message "{message}" '
        "and exc_info=True and stack_info=True"
    )
)
def log_with_both(
    logger_fixture: dict[str, typ.Any],
    exception_state: dict[str, typ.Any],
    level: str,
    message: str,
) -> None:
    logger = logger_fixture["logger"]

    if exception_state.get("active"):
        exc_type = exception_state.get("type", ValueError)
        exc_message = exception_state.get("message", "error")
        try:
            _raise_exception(exc_type, exc_message)
        except exc_type:
            logger_fixture["output"] = logger.log(
                level, message, exc_info=True, stack_info=True
            )
    else:
        logger_fixture["output"] = logger.log(
            level, message, exc_info=True, stack_info=True
        )


@then(parsers.parse('the formatted output contains "{text}"'))
def output_contains(logger_fixture: dict[str, typ.Any], text: str) -> None:
    output = logger_fixture["output"]
    assert output is not None, "Expected output but got None"
    assert text in output, f"Expected '{text}' in output: {output}"


@then(parsers.parse('the formatted output equals "{expected}"'))
def output_equals(logger_fixture: dict[str, typ.Any], expected: str) -> None:
    output = logger_fixture["output"]
    assert output == expected, f"Expected '{expected}' but got '{output}'"


@then("the formatted output matches snapshot")
def output_matches_snapshot(
    logger_fixture: dict[str, typ.Any], snapshot: SnapshotAssertion
) -> None:
    output = logger_fixture["output"]
    # Normalise paths and line numbers for snapshot stability
    normalised = _normalise_traceback_output(output)
    assert normalised == snapshot


def _normalise_traceback_output(output: str | None) -> str:
    """Normalise traceback output for snapshot comparison.

    Replaces file paths and line numbers with stable placeholders.
    """
    if output is None:
        return ""

    # Replace file paths with placeholder
    result = re.sub(
        r'File "[^"]+"',
        'File "<file>"',
        output,
    )
    # Replace line numbers
    return re.sub(r", line \d+,", ", line <N>,", result)
