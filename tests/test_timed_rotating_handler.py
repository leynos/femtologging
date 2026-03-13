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
    from pathlib import Path


def test_timed_rotating_handler_defaults(tmp_path: Path) -> None:
    """Constructing with defaults should preserve the default schedule."""
    path = tmp_path / "timed.log"
    handler = FemtoTimedRotatingFileHandler(str(path))
    try:
        assert handler.when == "H", "default cadence must be hourly"
        assert handler.interval == 1, "default interval must be one"
        assert handler.backup_count == 0, "default retention must keep all backups"
        assert handler.utc is False, "default schedule must use local time"
        assert handler.at_time is None, "hourly rotation must not carry at_time"
    finally:
        handler.close()


def test_timed_rotating_handler_accepts_options(tmp_path: Path) -> None:
    """Timed handler options should configure schedule and queue settings."""
    path = tmp_path / "timed.log"
    options = TimedHandlerOptions(
        capacity=32,
        flush_interval=2,
        policy="block",
        when="MIDNIGHT",
        interval=1,
        backup_count=3,
        utc=True,
        at_time=dt.time(6, 30, 0),
    )
    handler = FemtoTimedRotatingFileHandler(str(path), options=options)
    try:
        assert handler.when == "MIDNIGHT", "when must round-trip"
        assert handler.interval == 1, "interval must round-trip"
        assert handler.backup_count == 3, "backup_count must round-trip"
        assert handler.utc is True, "utc flag must round-trip"
        assert handler.at_time == "06:30:00", "at_time must format consistently"
        handler.handle("timed", "INFO", "probe message")
        assert isinstance(handler.flush(), bool), "flush must return a boolean"
    finally:
        handler.close()


def test_timed_rotating_handler_rejects_invalid_when(tmp_path: Path) -> None:
    """Unsupported schedule values should fail fast."""
    path = tmp_path / "timed.log"
    with pytest.raises(ValueError, match=TIMED_ROTATION_VALIDATION_MSG):
        FemtoTimedRotatingFileHandler(
            str(path),
            options=TimedHandlerOptions(when="fortnight"),
        )


def test_timed_rotating_handler_rejects_at_time_for_hourly(tmp_path: Path) -> None:
    """Hour-based rotation should reject at_time."""
    path = tmp_path / "timed.log"
    with pytest.raises(
        ValueError,
        match=("at_time is only supported for daily, midnight, and weekday rotation"),
    ):
        FemtoTimedRotatingFileHandler(
            str(path),
            options=TimedHandlerOptions(when="H", at_time=dt.time(8, 15, 0)),
        )


def test_timed_handler_options_reject_timezone_aware_time() -> None:
    """Timezone-aware at_time values should be rejected."""
    with pytest.raises(ValueError, match="timezone-naive"):
        TimedHandlerOptions(at_time=dt.time(9, 0, 0, tzinfo=dt.UTC))
