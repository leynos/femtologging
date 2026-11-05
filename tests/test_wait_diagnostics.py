"""Tests for the bounded file-handler wait diagnostics.

Issue #441 releases the interpreter lock around the ``handle()``, ``flush()``,
and ``close()`` waits. Each release also records a fixed-cardinality counter so
an operator can tell an accepted record from a dropped one, or an acknowledged
flush from one that found the handler closed, without scraping unbounded
labels.
"""

from __future__ import annotations

import collections.abc as cabc
import threading
import typing as typ
from contextlib import closing
from pathlib import Path

import pytest

from femtologging import (
    FemtoFileHandler,
    FemtoRotatingFileHandler,
    FemtoTimedRotatingFileHandler,
    FileHandlerBuilder,
    HandlerOptions,
    OverflowPolicy,
    TimedHandlerOptions,
)


class Handler(typ.Protocol):
    """The Python-facing surface shared by every worker-backed file handler."""

    def handle(self, logger: str, level: str, message: str) -> None:
        """Enqueue one record, waiting per the handler's overflow policy."""

    def flush(self) -> bool:
        """Flush queued records and report whether the worker acknowledged."""

    def close(self) -> None:
        """Stop the worker, waiting no longer than the close deadline."""

    def wait_diagnostics(self) -> dict[str, int]:
        """Return the fixed-key wait counters."""


type HandlerFactory = cabc.Callable[[Path], Handler]

#: The complete key set ``wait_diagnostics()`` exposes: one counter per
#: outcome plus the two duration totals. Nothing here carries a logger name,
#: path, message, or raw timeout value.
EXPECTED_KEYS = frozenset({
    "handle_accepted",
    "handle_queue_full",
    "handle_timed_out",
    "handle_closed",
    "handle_failed",
    "flush_acknowledged",
    "flush_timed_out",
    "flush_closed",
    "flush_failed",
    "close_completed",
    "close_timed_out",
    "close_panicked",
    "total_wait_micros",
    "max_wait_micros",
})


def _file_handler(path: Path) -> FemtoFileHandler:
    return FemtoFileHandler(str(path))


def _rotating_handler(path: Path) -> FemtoRotatingFileHandler:
    return FemtoRotatingFileHandler(str(path), HandlerOptions())


def _timed_rotating_handler(path: Path) -> FemtoTimedRotatingFileHandler:
    return FemtoTimedRotatingFileHandler(str(path), TimedHandlerOptions())


@pytest.mark.parametrize(
    "factory",
    [_file_handler, _rotating_handler, _timed_rotating_handler],
    ids=["file", "rotating", "timed_rotating"],
)
def test_wait_diagnostics_is_zeroed_and_bounded(
    tmp_path: Path, factory: HandlerFactory
) -> None:
    """A fresh handler exposes the fixed key set with every counter at zero."""
    with closing(factory(tmp_path / "out.log")) as handler:
        diagnostics = handler.wait_diagnostics()

    assert set(diagnostics) == EXPECTED_KEYS, (
        f"wait_diagnostics() exposed an unexpected key set: {sorted(diagnostics)!r}"
    )
    assert all(value == 0 for value in diagnostics.values()), (
        f"a fresh handler must not report waits: {diagnostics!r}"
    )


@pytest.mark.parametrize(
    "factory",
    [_file_handler, _rotating_handler, _timed_rotating_handler],
    ids=["file", "rotating", "timed_rotating"],
)
def test_successful_operations_record_their_outcomes(
    tmp_path: Path, factory: HandlerFactory
) -> None:
    """Handle, flush, and close each increment one bounded counter."""
    with closing(factory(tmp_path / "out.log")) as handler:
        handler.handle("core", "INFO", "one")
        diagnostics = handler.wait_diagnostics()
        assert diagnostics["handle_accepted"] == 1, (
            f"an accepted handle must be counted once: {diagnostics!r}"
        )

        assert handler.flush() is True, "an idle handler did not acknowledge its flush"
        diagnostics = handler.wait_diagnostics()
        assert diagnostics["flush_acknowledged"] == 1, (
            f"an acknowledged flush must be counted once: {diagnostics!r}"
        )

        handler.close()
        diagnostics = handler.wait_diagnostics()

    assert diagnostics["close_completed"] == 1, (
        f"a completed close must be counted once: {diagnostics!r}"
    )
    assert diagnostics["total_wait_micros"] >= diagnostics["max_wait_micros"] >= 0, (
        f"total wait time must bound the longest wait: {diagnostics!r}"
    )


@pytest.mark.parametrize(
    ("factory", "handle_error"),
    [
        (_file_handler, RuntimeError),
        (_rotating_handler, RuntimeError),
        (_timed_rotating_handler, ValueError),
    ],
    ids=["file", "rotating", "timed_rotating"],
)
def test_operations_after_close_record_closed_outcomes(
    tmp_path: Path,
    factory: HandlerFactory,
    handle_error: type[Exception],
) -> None:
    """A closed handler counts the rejected handle and the failed flush."""
    with closing(factory(tmp_path / "out.log")) as handler:
        handler.close()

        with pytest.raises(handle_error):
            handler.handle("core", "INFO", "late")
        diagnostics = handler.wait_diagnostics()
        assert diagnostics["handle_closed"] == 1, (
            f"a rejected post-close handle must be counted as closed: {diagnostics!r}"
        )

        assert handler.flush() is False, "flush() succeeded after the handler closed"
        diagnostics = handler.wait_diagnostics()
        assert diagnostics["flush_closed"] == 1, (
            f"a post-close flush must be counted as closed: {diagnostics!r}"
        )


def test_queue_full_under_drop_policy_is_counted(tmp_path: Path) -> None:
    """A dropped record is counted separately from an accepted one."""
    worker_started = threading.Event()
    release_worker = threading.Event()

    def blocking_formatter(record: cabc.Mapping[str, object]) -> str:
        worker_started.set()
        release_worker.wait(timeout=5)
        return f"{record['logger']} [{record['level']}] {record['message']}"

    handler = (
        FileHandlerBuilder(str(tmp_path / "full.log"))
        .with_capacity(1)
        .with_flush_after_records(10_000)
        .with_overflow_policy(OverflowPolicy.drop())
        .with_formatter(blocking_formatter)
        .build()
    )

    try:
        # The worker takes the first record and parks inside the formatter, so
        # the queue is empty when the second record arrives and full when the
        # third does.
        handler.handle("core", "INFO", "first")
        assert worker_started.wait(5), "the worker never reached the formatter"
        handler.handle("core", "INFO", "second")

        with pytest.raises(RuntimeError):
            handler.handle("core", "INFO", "third")

        diagnostics = handler.wait_diagnostics()
        assert diagnostics["handle_accepted"] == 2, (
            f"both queued records must be counted as accepted: {diagnostics!r}"
        )
        assert diagnostics["handle_queue_full"] == 1, (
            f"the dropped record must be counted as queue-full: {diagnostics!r}"
        )
    finally:
        release_worker.set()
        handler.close()
