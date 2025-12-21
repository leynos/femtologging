"""Shared helpers for the test suite."""

from __future__ import annotations

import time
import typing as typ

import pytest

if typ.TYPE_CHECKING:
    from pathlib import Path

# Polling interval for file content verification (seconds).
_POLL_INTERVAL_SECONDS: float = 0.01


def poll_file_for_text(path: Path, expected: str, timeout: float = 1.0) -> str:
    """Poll a file until it contains expected text or timeout expires.

    Parameters
    ----------
    path : Path
        Path to the file to poll.
    expected : str
        Text substring that must appear in the file contents.
    timeout : float, optional
        Maximum time to wait in seconds (default: 1.0).

    Returns
    -------
    str
        The file contents once the expected text is found.

    Raises
    ------
    Failed
        Via pytest.fail() if the expected text is not found within the timeout.

    Examples
    --------
    >>> contents = poll_file_for_text(log_path, "ERROR", timeout=2.0)
    >>> assert "ERROR: Something failed" in contents

    """
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            contents = path.read_text()
            if expected in contents:
                return contents
        time.sleep(_POLL_INTERVAL_SECONDS)
    pytest.fail(f"log file did not contain '{expected}' within {timeout}s")
