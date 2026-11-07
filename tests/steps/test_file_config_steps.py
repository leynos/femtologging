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
    """Apply fileConfig to the file at the given path."""
    fileConfig(Path(config_path))


@when(
    parsers.parse('I attempt to configure fileConfig from "{config_path}"'),
    target_fixture="config_error",
)
def when_file_config_fails(config_path: str) -> ValueError:
    """Apply fileConfig expecting a missing-class ValueError and return it."""
    with pytest.raises(ValueError, match="missing class") as err:
        fileConfig(Path(config_path))
    return err.value


@then("fileConfig raises ValueError")
def then_file_config_raises(config_error: ValueError) -> None:
    """Assert the fileConfig ValueError message contains "missing class"."""
    # The @when step already asserted the exception type via pytest.raises; this
    # step pins the operator-facing message so the diagnosis stays actionable.
    assert "missing class" in str(config_error), (
        f'fileConfig error text must contain "missing class", got: {config_error!r}'
    )
