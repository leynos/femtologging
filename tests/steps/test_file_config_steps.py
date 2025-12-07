"""Behaviour-driven tests for femtologging.fileConfig."""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import parsers, scenarios, then, when

from femtologging import fileConfig, get_logger

if typ.TYPE_CHECKING:
    from syrupy.assertion import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "file_config.feature"))


@when(parsers.parse('I configure fileConfig from "{config_path}"'))
def when_file_config(config_path: str) -> None:
    fileConfig(Path(config_path))


@when(
    parsers.parse('I attempt to configure fileConfig from "{config_path}"'),
    target_fixture="config_error",
)
def when_file_config_fails(config_path: str) -> ValueError:
    with pytest.raises(ValueError, match="missing class") as err:
        fileConfig(Path(config_path))
    return err.value


@then(parsers.parse('logging "{message}" at "{level}" from root matches snapshot'))
def then_log_matches_snapshot(
    message: str, level: str, snapshot: SnapshotAssertion
) -> None:
    logger = get_logger("root")
    formatted = logger.log(level, message)
    if level.upper() == "DEBUG":
        assert formatted is None
    else:
        assert formatted is not None
        assert formatted == snapshot


@then("fileConfig raises ValueError")
def then_file_config_raises(config_error: ValueError) -> None:
    assert isinstance(config_error, ValueError)
