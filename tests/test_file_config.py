"""Tests for femtologging.fileConfig."""

from __future__ import annotations

import time
import typing as typ
from os import fsencode
from pathlib import Path

import pytest

from femtologging import fileConfig, get_logger, reset_manager

if typ.TYPE_CHECKING:
    import collections.abc as cabc


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    """Reset the global manager around each test."""
    reset_manager()
    yield
    reset_manager()


def _write_file_handler_ini(config_path: Path, log_path: Path) -> None:
    config_path.write_text(
        "[loggers]\nkeys = root\n\n"
        "[handlers]\nkeys = file\n\n"
        "[handler_file]\nclass = femtologging.FileHandler\n"
        f"args = ('{log_path}',)\n\n"
        "[logger_root]\nlevel = INFO\nhandlers = file\n",
        encoding="utf-8",
    )


def _read_log_when_written(path: Path, expected: str, timeout: float = 1.5) -> str:
    """Return the log file's contents once *expected* has been flushed to it."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            contents = path.read_text()
            if expected in contents:
                return contents
        time.sleep(0.01)
    msg = f"log file {path} not written in time"
    raise TimeoutError(msg)


def _assert_single_root_record(log_path: Path, message: str, context: str) -> None:
    """Assert *log_path* holds exactly one INFO record carrying *message*."""
    contents = _read_log_when_written(log_path, message)
    assert contents.splitlines() == [f"root [INFO] {message}"], (
        f"{context}: the configured file handler must receive exactly one "
        f"formatted root record, but {log_path} contains {contents!r}"
    )


def test_file_config_expands_defaults(tmp_path: Path) -> None:
    """INIs honour defaults passed to fileConfig for placeholder substitution."""
    ini_path = Path("tests/data/fileconfig_defaults.ini")
    defaults = {"logdir": str(tmp_path)}

    fileConfig(ini_path, defaults=defaults)
    get_logger("root").log("INFO", "defaults work")

    _assert_single_root_record(
        tmp_path / "app.log",
        "defaults work",
        "fileConfig should substitute the 'logdir' default into the handler args",
    )


def test_file_config_rejects_handler_level(tmp_path: Path) -> None:
    """Handler level specification should be rejected by fileConfig."""
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
    "build_input",
    [
        pytest.param(str, id="str"),
        pytest.param(lambda path: path, id="path"),
        pytest.param(lambda path: fsencode(str(path)), id="bytes"),
    ],
)
def test_file_config_accepts_common_path_types(
    tmp_path: Path,
    build_input: cabc.Callable[[Path], str | Path | bytes],
) -> None:
    """FileConfig accepts str, Path, and bytes path inputs."""
    ini_path = tmp_path / "path_types.ini"
    log_path = tmp_path / "path_types.log"
    _write_file_handler_ini(ini_path, log_path)

    fileConfig(build_input(ini_path))
    get_logger("root").log("INFO", "path types work")

    _assert_single_root_record(
        log_path,
        "path types work",
        "fileConfig must load the same INI whichever path spelling is supplied",
    )
