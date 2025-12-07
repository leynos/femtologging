"""Behaviour-driven tests for femtologging.fileConfig."""

from __future__ import annotations

from pathlib import Path

import pytest
from pytest_bdd import parsers, scenarios, then, when

from femtologging import fileConfig

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


@then("fileConfig raises ValueError")
def then_file_config_raises(config_error: ValueError) -> None:
    # The @when step already validated the error message via pytest.raises(match=...).
    # This step confirms the fixture captured an exception for BDD completeness.
    assert config_error is not None
