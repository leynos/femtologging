"""Behaviour-driven tests for femtologging.fileConfig."""

from __future__ import annotations

import time
from os import fsencode
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import fileConfig, get_logger, reset_manager

scenarios("features/file_config.feature")


def _write_file_handler_ini(config_path: Path, log_path: Path) -> None:
    config_path.write_text(
        "[loggers]\nkeys = root\n\n"
        "[handlers]\nkeys = file\n\n"
        "[handler_file]\nclass = femtologging.FileHandler\n"
        f"args = ('{log_path}',)\n\n"
        "[logger_root]\nlevel = INFO\nhandlers = file\n",
        encoding="utf-8",
    )


def _wait_for_log_line(path: Path, expected: str, timeout: float = 1.5) -> str:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            contents = path.read_text()
            if expected in contents:
                return contents
        time.sleep(0.01)
    pytest.fail(f"log file {path} not written in time")


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
    reset_manager()
    ini_path = Path("tests/data/fileconfig_defaults.ini")
    defaults = {"logdir": str(tmp_path)}
    fileConfig(ini_path, defaults=defaults)
    logger = get_logger("root")
    logger.log("INFO", "defaults work")
    log_path = tmp_path / "app.log"
    contents = _wait_for_log_line(log_path, "defaults work")
    assert "defaults work" in contents


def test_file_config_rejects_handler_level(tmp_path: Path) -> None:
    reset_manager()
    ini = tmp_path / "bad.ini"
    ini.write_text(
        "[loggers]\nkeys = root\n\n[handlers]\nkeys = h\n\n"
        "[handler_h]\nclass = femtologging.StreamHandler\nlevel = INFO\n\n"
        "[logger_root]\nhandlers = h\n",
        encoding="utf-8",
    )
    with pytest.raises(ValueError, match="handler level is not supported"):
        fileConfig(ini)


@pytest.mark.parametrize(
    "path_builder",
    [
        pytest.param(str, id="str"),
        pytest.param(lambda path: path, id="path"),
        pytest.param(lambda path: fsencode(str(path)), id="bytes"),
    ],
)
def test_file_config_accepts_common_path_types(tmp_path: Path, path_builder) -> None:
    reset_manager()
    ini_path = tmp_path / "path_types.ini"
    log_path = tmp_path / "path_types.log"
    _write_file_handler_ini(ini_path, log_path)

    fileConfig(path_builder(ini_path))

    logger = get_logger("root")
    logger.log("INFO", "path types work")

    contents = _wait_for_log_line(log_path, "path types work")

    assert "path types work" in contents
