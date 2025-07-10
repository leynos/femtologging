# pyright: reportMissingImports=false
"""Tests for :class:`FemtoLogger`."""

from __future__ import annotations

import pytest  # pyright: ignore[reportMissingImports]
from femtologging import FemtoLogger


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
