"""BDD steps for exception info logging scenarios."""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import FemtoLogger

from .conftest import normalise_traceback_output

if typ.TYPE_CHECKING:
    from syrupy import SnapshotAssertion


class LoggerFixture(typ.TypedDict, total=False):
    """State storage for logger fixture."""

    logger: FemtoLogger | None
    output: str | None


class ExceptionState(typ.TypedDict, total=False):
    """State storage for exception fixture."""

    instance: BaseException | None
    active: bool
    type: type[BaseException] | None
    message: str | None
    chained: bool
    group: bool


FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "exc_info.feature"))


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
def logger_fixture() -> LoggerFixture:
    """Storage for the logger and related state."""
    return {"logger": None, "output": None}


@pytest.fixture
def exception_state() -> ExceptionState:
    """Storage for exception state."""
    return {"instance": None, "active": False}


@given(parsers.parse('a logger named "{name}"'))
def create_logger(logger_fixture: LoggerFixture, name: str) -> None:
    logger_fixture["logger"] = FemtoLogger(name)


@given(parsers.parse('an active ValueError exception with message "{message}"'))
def set_active_value_error(exception_state: ExceptionState, message: str) -> None:
    exception_state["active"] = True
    exception_state["type"] = ValueError
    exception_state["message"] = message


@given(parsers.parse('an exception instance KeyError with message "{message}"'))
def create_key_error_instance(exception_state: ExceptionState, message: str) -> None:
    exception_state["instance"] = KeyError(message)


@given("an exception chain: RuntimeError from OSError")
def create_chained_exception(exception_state: ExceptionState) -> None:
    exception_state["active"] = True
    exception_state["chained"] = True


@given("an exception group with ValueError and TypeError")
def create_exception_group(exception_state: ExceptionState) -> None:
    exception_state["active"] = True
    exception_state["group"] = True


def _log_with_chained_exception(
    logger: FemtoLogger, level: str, message: str
) -> str | None:
    """Log a message with a chained exception (RuntimeError from OSError)."""
    try:
        _raise_chained_exception()
    except RuntimeError:
        return logger.log(level, message, exc_info=True)
    return None  # unreachable, but keeps type checker happy


def _log_with_exception_group(
    logger: FemtoLogger, level: str, message: str
) -> str | None:
    """Log a message with an exception group."""
    try:
        _raise_exception_group()
    except ExceptionGroup:
        return logger.log(level, message, exc_info=True)
    return None  # unreachable, but keeps type checker happy


def _raise_dynamic_exception(
    exc_type: type[BaseException], exc_message: str
) -> typ.NoReturn:
    """Raise an exception of the given type with the given message."""
    raise exc_type(exc_message)


def _log_with_simple_exception(
    logger: FemtoLogger,
    level: str,
    message: str,
    exception_state: ExceptionState,
) -> str | None:
    """Log a message with a simple exception."""
    exc_type = exception_state.get("type", ValueError)
    exc_message = exception_state.get("message", "error")
    assert exc_type is not None, "Exception type must be set"
    assert exc_message is not None, "Exception message must be set"
    try:
        _raise_dynamic_exception(exc_type, exc_message)
    except exc_type:
        return logger.log(level, message, exc_info=True)
    return None  # unreachable, but keeps type checker happy


@when(parsers.parse('I log at {level} with message "{message}" and exc_info=True'))
def log_with_exc_info_true(
    logger_fixture: LoggerFixture,
    exception_state: ExceptionState,
    level: str,
    message: str,
) -> None:
    logger = logger_fixture["logger"]
    assert logger is not None, "Logger must be initialized"

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
    logger_fixture: LoggerFixture,
    exception_state: ExceptionState,
    level: str,
    message: str,
) -> None:
    logger = logger_fixture["logger"]
    assert logger is not None, "Logger must be initialized"
    exc_instance = exception_state["instance"]
    logger_fixture["output"] = logger.log(level, message, exc_info=exc_instance)


@then(parsers.parse('the formatted output contains "{text}"'))
def output_contains(logger_fixture: LoggerFixture, text: str) -> None:
    output = logger_fixture["output"]
    assert output is not None, "Expected output but got None"
    assert text in output, f"Expected '{text}' in output: {output}"


@then(parsers.parse('the formatted output equals "{expected}"'))
def output_equals(logger_fixture: LoggerFixture, expected: str) -> None:
    output = logger_fixture["output"]
    assert output == expected, f"Expected '{expected}' but got '{output}'"


@then("the formatted output matches snapshot")
def output_matches_snapshot(
    logger_fixture: LoggerFixture, snapshot: SnapshotAssertion
) -> None:
    output = logger_fixture["output"]
    # Normalise paths and line numbers for snapshot stability
    normalised = normalise_traceback_output(output)
    assert normalised == snapshot
