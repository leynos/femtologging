"""Shared helpers for the test suite."""

from __future__ import annotations

import time
import typing as typ

if typ.TYPE_CHECKING:
    from pathlib import Path

# Polling interval for file content verification (seconds).
_POLL_INTERVAL_SECONDS: float = 0.01


class CapturedText(typ.Protocol):
    """The stderr text produced by one capture drain."""

    @property
    def err(self) -> str:
        """The text written to standard error."""


class CaptureReader(typ.Protocol):
    """Capture-fixture interface the polling helper depends on."""

    def readouterr(self) -> CapturedText:
        """Drain the capture buffers and return what they held."""


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
    AssertionError
        If the expected text is not found within the timeout.

    Examples
    --------
    >>> import pathlib
    >>> import tempfile
    >>> with tempfile.TemporaryDirectory() as directory:
    ...     log_path = pathlib.Path(directory) / "app.log"
    ...     _ = log_path.write_text("ERROR: Something failed")
    ...     contents = poll_file_for_text(log_path, "ERROR", timeout=2.0)
    >>> "ERROR: Something failed" in contents
    True

    """
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            contents = path.read_text()
            if expected in contents:
                return contents
        time.sleep(_POLL_INTERVAL_SECONDS)
    message = f"log file did not contain '{expected}' within {timeout}s"
    raise AssertionError(message)


def poll_captured_text(
    capture: CaptureReader, expected: str, *, timeout: float = 10.0
) -> str:
    r"""Poll a pytest capture fixture until its stderr contains expected text.

    Draining the capture is the only way to observe output the worker thread
    may still be writing, so the helper accumulates each drain and keeps
    polling instead of reading once. Callers use it after a ``flush()`` or
    ``close()`` that may have timed out, where a single read races the worker.

    Parameters
    ----------
    capture : CaptureReader
        Capture fixture, such as ``capfd`` or ``capsys``.
    expected : str
        Text substring that must appear in the captured stderr.
    timeout : float, optional
        Maximum time to wait in seconds (default: 10.0).

    Returns
    -------
    str
        The accumulated stderr once the expected text is found.

    Raises
    ------
    AssertionError
        If the expected text is not found within the timeout.

    Examples
    --------
    >>> class _Capture:
    ...     def __init__(self) -> None:
    ...         self._drained = False
    ...     def readouterr(self) -> object:
    ...         text = "" if self._drained else "hello\n"
    ...         self._drained = True
    ...         return type("_Result", (), {"err": text})()
    >>> captured = poll_captured_text(_Capture(), "hello\n")
    >>> "hello" in captured
    True

    """
    # A monotonic deadline keeps a wall-clock adjustment from stretching or
    # cutting the wait; only elapsed time is meant to matter here.
    deadline = time.monotonic() + timeout
    collected = ""
    while time.monotonic() < deadline:
        collected += capture.readouterr().err
        if expected in collected:
            return collected
        time.sleep(_POLL_INTERVAL_SECONDS)
    message = (
        f"captured stderr did not contain {expected!r} within {timeout}s; "
        f"got {collected!r}"
    )
    raise AssertionError(message)
