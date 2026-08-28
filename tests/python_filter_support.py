"""Shared scaffolding for the Python callback filter tests.

The record collector, sample filter objects, and the factory targets referenced
by ``dictConfig`` payloads live here so both the callback-filter tests and the
``dictConfig`` factory-resolution tests can share them by a stable dotted path.
"""

from __future__ import annotations

import contextvars
import logging
import threading
import time
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

REQUEST_ID: contextvars.ContextVar[str] = contextvars.ContextVar(
    "request_id", default=""
)
"""Correlation identifier read by the enrichment filters under test."""


class RecordCollector:
    """Collect full record payloads from ``handle_record``."""

    def __init__(self) -> None:
        """Initialize the collector state."""
        self.records: list[dict[str, typ.Any]] = []
        self._lock = threading.Lock()

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback handler required by femtologging validation."""

    def handle_record(self, record: dict[str, typ.Any]) -> None:
        """Store structured records for assertions."""
        with self._lock:
            self.records.append(record)


class PrefixRejectingFilter(logging.Filter):
    """Example stdlib-style filter object with a ``filter`` method."""

    def __init__(self, prefix: str) -> None:
        """Store the accepted logger-name prefix."""
        super().__init__()
        self.prefix = prefix

    def filter(self, record: logging.LogRecord) -> bool:
        """Allow only records whose logger starts with ``self.prefix``.

        Returns
        -------
        bool
            ``True`` when the record's logger name carries the prefix.
        """
        return record.name.startswith(self.prefix)


class ContextFilterFactory(logging.Filter):
    """Factory used by ``dictConfig`` filter tests."""

    def __init__(self, request_id: str) -> None:
        """Store the request ID to inject into accepted records."""
        super().__init__()
        self.request_id = request_id

    def filter(self, record: logging.LogRecord) -> bool:
        """Attach the configured request ID and accept the record.

        Returns
        -------
        bool
            Always ``True``; the filter only enriches.
        """
        record.request_id = self.request_id
        return True


NONCALLABLE_FACTORY = object()
"""Resolution target used to prove non-callable factories are rejected."""


def wait_for(
    condition: cabc.Callable[[], bool], description: str, timeout: float = 1.0
) -> None:
    """Poll ``condition`` until it holds, or fail after ``timeout`` seconds."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        if condition():
            return
        time.sleep(0.01)
    msg = f"timed out after {timeout}s waiting until {description}"
    raise AssertionError(msg)
