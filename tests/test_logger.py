# pyright: reportMissingImports=false
"""Tests for :class:`FemtoLogger`."""

from __future__ import annotations

import pytest  # pyright: ignore[reportMissingImports]
import collections.abc as cabc
from pathlib import Path
import typing

from femtologging import FemtoFileHandler, FemtoLogger

FileHandlerFactory = cabc.Callable[
    [Path, int, int], typing.ContextManager[FemtoFileHandler]
]


@pytest.mark.parametrize(
    "name, level, message, expected",
    [
        ("core", "INFO", "hello", "core [INFO] hello"),
        ("sys", "ERROR", "fail", "sys [ERROR] fail"),
        ("", "INFO", "empty name", " [INFO] empty name"),
        ("core", "INFO", "", "core [INFO] "),
        ("i18n", "INFO", "こんにちは世界", "i18n [INFO] こんにちは世界"),
        (
            "n" * 1000,
            "INFO",
            "m" * 1000,
            f"{'n' * 1000} [INFO] {'m' * 1000}",
        ),
    ],
)
def test_log_formats_message(
    name: str, level: str, message: str, expected: str
) -> None:
    logger = FemtoLogger(name)
    assert logger.log(level, message) == expected


def test_log_respects_logger_level() -> None:
    """Messages below the logger level should be ignored."""
    logger = FemtoLogger("core")
    logger.set_level("ERROR")
    assert logger.log("INFO", "ignored") is None
    assert logger.log("ERROR", "processed") == "core [ERROR] processed"


def test_level_parsing_and_filtering() -> None:
    """Verify log level parsing and filtering across variants."""
    logger = FemtoLogger("core")
    for lvl in ["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "CRITICAL"]:
        logger.set_level(lvl)
        assert logger.log(lvl, "ok") is not None

    logger.set_level("ERROR")
    assert logger.log("WARN", "drop") is None
    with pytest.raises(ValueError):
        logger.log("bogus", "drop")


def test_logger_add_handler(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """Records should be written to every attached handler."""
    path1 = tmp_path / "one.log"
    path2 = tmp_path / "two.log"
    with (
        file_handler_factory(path1, 8, 1) as h1,
        file_handler_factory(path2, 8, 1) as h2,
    ):
        logger = FemtoLogger("core")
        logger.add_handler(h1)
        logger.add_handler(h2)
        logger.log("INFO", "hello")
        del logger
    assert path1.read_text() == "core [INFO] hello\n"
    assert path2.read_text() == "core [INFO] hello\n"
