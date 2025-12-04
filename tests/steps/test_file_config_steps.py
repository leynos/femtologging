"""Behaviour-driven tests for femtologging.fileConfig."""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import fileConfig, get_logger, reset_manager

if typ.TYPE_CHECKING:
    from syrupy.assertion import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "file_config.feature"))


@given("the logging system is reset")
def given_logging_reset() -> None:
    reset_manager()


@when(parsers.parse('I configure fileConfig from "{config_path}"'))
def when_file_config(config_path: str) -> None:
    fileConfig(Path(config_path))


@when(
    parsers.parse('I attempt to configure fileConfig from "{config_path}"'),
    target_fixture="config_error",
)
def when_file_config_fails(config_path: str) -> BaseException:
    with pytest.raises(ValueError, match=r".*") as err:
        fileConfig(Path(config_path))
    return err.value


@then(parsers.parse('logging "{message}" at "{level}" from root matches snapshot'))
def then_log_matches_snapshot(
    message: str, level: str, snapshot: SnapshotAssertion
) -> None:
    logger = get_logger("root")
    assert logger.log(level, message) == snapshot


@then("fileConfig raises ValueError")
def then_file_config_raises(config_error: ValueError) -> None:
    assert isinstance(config_error, ValueError)
