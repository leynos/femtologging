"""Tests for FemtoTimedRotatingFileHandler Python bindings."""

from __future__ import annotations

import datetime as dt
import typing as typ

import pytest

from femtologging import (
    TIMED_ROTATION_VALIDATION_MSG,
    FemtoTimedRotatingFileHandler,
    TimedHandlerOptions,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

    type TimedHandlerFactory = cabc.Callable[
        [TimedHandlerOptions | None], FemtoTimedRotatingFileHandler
    ]

# The rotation schedule attributes exposed on the handler, in a stable order.
SCHEDULE_ATTRIBUTES = ("when", "interval", "backup_count", "utc", "at_time")


@pytest.fixture(name="open_timed_handler")
def fixture_open_timed_handler(
    tmp_path: Path,
) -> cabc.Iterator[TimedHandlerFactory]:
    """Open timed rotating handlers and close them once the test finishes."""
    handlers: list[FemtoTimedRotatingFileHandler] = []

    def open_handler(
        options: TimedHandlerOptions | None = None,
    ) -> FemtoTimedRotatingFileHandler:
        path = str(tmp_path / "timed.log")
        handler = (
            FemtoTimedRotatingFileHandler(path)
            if options is None
            else FemtoTimedRotatingFileHandler(path, options=options)
        )
        handlers.append(handler)
        return handler

    try:
        yield open_handler
    finally:
        for handler in handlers:
            handler.close()


def _assert_schedule(
    handler: FemtoTimedRotatingFileHandler,
    expected: dict[str, object],
    context: str,
) -> None:
    """Assert every schedule attribute of *handler* matches *expected*."""
    actual = {name: getattr(handler, name) for name in SCHEDULE_ATTRIBUTES}
    assert actual == expected, (
        f"{context}: the rotation schedule must match the requested "
        f"configuration; expected {expected}, got {actual}"
    )


def test_timed_rotating_handler_defaults(
    open_timed_handler: TimedHandlerFactory,
) -> None:
    """Constructing with defaults should preserve the default schedule."""
    handler = open_timed_handler(None)

    _assert_schedule(
        handler,
        {
            "when": "H",
            "interval": 1,
            "backup_count": 0,
            "utc": False,
            "at_time": None,
        },
        "an unconfigured handler rotates hourly in local time, keeping all backups",
    )


def test_timed_rotating_handler_accepts_options(
    open_timed_handler: TimedHandlerFactory,
) -> None:
    """Timed handler options configure schedule and queue settings."""
    handler = open_timed_handler(
        TimedHandlerOptions(
            capacity=32,
            flush_interval=2,
            policy="block",
            when="MIDNIGHT",
            interval=1,
            backup_count=3,
            utc=True,
            at_time=dt.time(6, 30, 0),
        )
    )

    _assert_schedule(
        handler,
        {
            "when": "MIDNIGHT",
            "interval": 1,
            "backup_count": 3,
            "utc": True,
            "at_time": "06:30:00",
        },
        "TimedHandlerOptions must round-trip onto the handler",
    )
    handler.handle("timed", "INFO", "probe message")
    assert handler.flush() is True, (
        "flush must report success after a record has been queued"
    )


def test_timed_rotating_handler_rejects_invalid_when(tmp_path: Path) -> None:
    """Unsupported schedule values should fail fast."""
    with pytest.raises(ValueError, match=TIMED_ROTATION_VALIDATION_MSG):
        FemtoTimedRotatingFileHandler(
            str(tmp_path / "timed.log"),
            options=TimedHandlerOptions(when="fortnight"),
        )


def test_timed_rotating_handler_rejects_at_time_for_hourly(tmp_path: Path) -> None:
    """Hour-based rotation should reject at_time."""
    with pytest.raises(
        ValueError,
        match="at_time is only supported for daily, midnight, and weekday rotation",
    ):
        FemtoTimedRotatingFileHandler(
            str(tmp_path / "timed.log"),
            options=TimedHandlerOptions(when="H", at_time=dt.time(8, 15, 0)),
        )


def test_timed_handler_options_reject_timezone_aware_time() -> None:
    """Timezone-aware at_time values should be rejected."""
    with pytest.raises(ValueError, match="timezone-naive"):
        TimedHandlerOptions(at_time=dt.time(9, 0, 0, tzinfo=dt.UTC))
