"""Shared helpers for the test suite."""

from __future__ import annotations

import time
import typing as typ

import pytest

if typ.TYPE_CHECKING:
    from pathlib import Path


def _poll_file_for_text(path: Path, expected: str, timeout: float = 1.0) -> str:
    """Poll a file until it contains expected text or timeout expires."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            contents = path.read_text()
            if expected in contents:
                return contents
        time.sleep(0.01)
    pytest.fail(f"log file did not contain '{expected}' within {timeout}s")
