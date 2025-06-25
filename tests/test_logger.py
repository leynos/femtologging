"""Tests for :class:`FemtoLogger`."""

from __future__ import annotations

import pytest
from femtologging import FemtoLogger


@pytest.mark.parametrize(
    "name, level, message, expected",
    [
        ("core", "INFO", "hello", "core: INFO - hello"),
        ("sys", "ERROR", "fail", "sys: ERROR - fail"),
    ],
)
def test_log_formats_message(
    name: str, level: str, message: str, expected: str
) -> None:
    logger = FemtoLogger(name)
    assert logger.log(level, message) == expected
