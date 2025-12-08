"""Tests for femtologging.fileConfig."""

from __future__ import annotations

import time
import typing as typ
from os import fsencode
from pathlib import Path

import pytest

from femtologging import fileConfig, get_logger, reset_manager


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
    msg = f"log file {path} not written in time"
    raise TimeoutError(msg)


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
    """Handler level specification should be rejected by fileConfig."""
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
def test_file_config_accepts_common_path_types(
    tmp_path: Path, path_builder: typ.Callable[[Path], str | Path | bytes]
) -> None:
    """FileConfig accepts str, Path, and bytes path inputs."""
    reset_manager()
    ini_path = tmp_path / "path_types.ini"
    log_path = tmp_path / "path_types.log"
    _write_file_handler_ini(ini_path, log_path)

    fileConfig(path_builder(ini_path))

    logger = get_logger("root")
    logger.log("INFO", "path types work")

    contents = _wait_for_log_line(log_path, "path types work")

    assert "path types work" in contents
