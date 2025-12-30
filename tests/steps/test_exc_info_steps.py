"""BDD steps for exception info logging scenarios."""

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

scenarios(str(FEATURES / "exc_info.feature"))


def _raise_exception(exc_type: type[BaseException], message: str) -> None:
    """Raise an exception of the given type with the given message."""
    raise exc_type(message)


def _raise_os_error() -> None:
    """Raise an OSError."""
    raise OSError


def _raise_chained_exception() -> None:
    """Raise a chained RuntimeError from OSError."""
    try:
        _raise_os_error()
    except OSError as e:
        raise RuntimeError from e


def _raise_exception_group() -> None:
    """Raise an ExceptionGroup with ValueError and TypeError."""
    msg = "multiple errors"
    raise ExceptionGroup(msg, [ValueError(), TypeError()])


@pytest.fixture
def logger_fixture() -> dict[str, typ.Any]:
    """Storage for the logger and related state."""
    return {"logger": None, "output": None}


@pytest.fixture
def exception_state() -> dict[str, typ.Any]:
    """Storage for exception state."""
    return {"instance": None, "active": False}


@given(parsers.parse('a logger named "{name}"'))
def create_logger(logger_fixture: dict[str, typ.Any], name: str) -> None:
    logger_fixture["logger"] = FemtoLogger(name)


@given(parsers.parse('an active ValueError exception with message "{message}"'))
def set_active_value_error(exception_state: dict[str, typ.Any], message: str) -> None:
    exception_state["active"] = True
    exception_state["type"] = ValueError
    exception_state["message"] = message


@given(parsers.parse('an exception instance KeyError with message "{message}"'))
def create_key_error_instance(
    exception_state: dict[str, typ.Any], message: str
) -> None:
    exception_state["instance"] = KeyError(message)


@given("an exception chain: RuntimeError from OSError")
def create_chained_exception(exception_state: dict[str, typ.Any]) -> None:
    exception_state["active"] = True
    exception_state["chained"] = True


@given("an exception group with ValueError and TypeError")
def create_exception_group(exception_state: dict[str, typ.Any]) -> None:
    exception_state["active"] = True
    exception_state["group"] = True


def _log_with_chained_exception(logger: FemtoLogger, level: str, message: str) -> str:
    """Log a message with a chained exception (RuntimeError from OSError)."""
    try:
        _raise_chained_exception()
    except RuntimeError:
        return logger.log(level, message, exc_info=True)
    return ""  # unreachable, but keeps type checker happy


def _log_with_exception_group(logger: FemtoLogger, level: str, message: str) -> str:
    """Log a message with an exception group."""
    try:
        _raise_exception_group()
    except ExceptionGroup:
        return logger.log(level, message, exc_info=True)
    return ""  # unreachable, but keeps type checker happy


def _log_with_simple_exception(
    logger: FemtoLogger,
    level: str,
    message: str,
    exception_state: dict[str, typ.Any],
) -> str:
    """Log a message with a simple exception."""
    exc_type = exception_state.get("type", ValueError)
    exc_message = exception_state.get("message", "error")
    try:
        _raise_exception(exc_type, exc_message)
    except exc_type:
        return logger.log(level, message, exc_info=True)
    return ""  # unreachable, but keeps type checker happy


@when(parsers.parse('I log at {level} with message "{message}" and exc_info=True'))
def log_with_exc_info_true(
    logger_fixture: dict[str, typ.Any],
    exception_state: dict[str, typ.Any],
    level: str,
    message: str,
) -> None:
    logger = logger_fixture["logger"]

    if not exception_state.get("active"):
        logger_fixture["output"] = logger.log(level, message, exc_info=True)
        return

    if exception_state.get("chained"):
        logger_fixture["output"] = _log_with_chained_exception(logger, level, message)
    elif exception_state.get("group"):
        logger_fixture["output"] = _log_with_exception_group(logger, level, message)
    else:
        logger_fixture["output"] = _log_with_simple_exception(
            logger, level, message, exception_state
        )


@when(
    parsers.parse(
        'I log at {level} with message "{message}" and exc_info as the instance'
    )
)
def log_with_exc_info_instance(
    logger_fixture: dict[str, typ.Any],
    exception_state: dict[str, typ.Any],
    level: str,
    message: str,
) -> None:
    logger = logger_fixture["logger"]
    exc_instance = exception_state["instance"]
    logger_fixture["output"] = logger.log(level, message, exc_info=exc_instance)


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
        'File "<test>"',
        output,
    )
    # Replace line numbers
    return re.sub(r", line \d+,", ", line <N>,", result)
