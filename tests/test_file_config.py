"""Behaviour-driven tests for femtologging.fileConfig."""

from __future__ import annotations

from pathlib import Path
import time

from femtologging import fileConfig, get_logger, reset_manager
import pytest
from pytest_bdd import given, parsers, scenarios, then, when

scenarios("features/file_config.feature")


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
    with pytest.raises(ValueError) as err:
        fileConfig(Path(config_path))
    return err.value


@then(parsers.parse('logging "{message}" at "{level}" from root matches snapshot'))
def then_log_matches_snapshot(message: str, level: str, snapshot) -> None:
    logger = get_logger("root")
    assert logger.log(level, message) == snapshot


@then("fileConfig raises ValueError")
def then_file_config_raises(config_error: ValueError) -> None:
    assert isinstance(config_error, ValueError)


def test_file_config_expands_defaults(tmp_path: Path) -> None:
    """INIs honour defaults passed to fileConfig for placeholder substitution."""

    ini_path = Path("tests/data/fileconfig_defaults.ini")
    defaults = {"logdir": str(tmp_path)}
    fileConfig(ini_path, defaults=defaults)
    logger = get_logger("root")
    logger.log("INFO", "defaults work")
    log_path = tmp_path / "app.log"
    deadline = time.time() + 1.5
    while time.time() < deadline:
        if log_path.exists() and "defaults work" in log_path.read_text():
            break
        time.sleep(0.01)
    else:
        pytest.fail("log file not written in time")


def test_file_config_rejects_handler_level(tmp_path: Path) -> None:
    ini = tmp_path / "bad.ini"
    ini.write_text(
        "[loggers]\nkeys = root\n\n[handlers]\nkeys = h\n\n"
        "[handler_h]\nclass = femtologging.StreamHandler\nlevel = INFO\n\n"
        "[logger_root]\nhandlers = h\n",
        encoding="utf-8",
    )
    with pytest.raises(ValueError, match="handler level is not supported"):
        fileConfig(ini)
