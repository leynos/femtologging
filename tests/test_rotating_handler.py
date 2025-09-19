"""Unit tests covering the rotating file handler Python bindings."""

from __future__ import annotations

import pathlib

import pytest

from femtologging import FemtoRotatingFileHandler, HandlerOptions


@pytest.fixture(name="log_path")
def fixture_log_path(tmp_path: pathlib.Path) -> pathlib.Path:
    """Provide a unique log file path for rotating handler tests."""

    return tmp_path / "rotating.log"


def _close_handler(handler: FemtoRotatingFileHandler) -> None:
    """Ensure handlers created in tests release their resources."""

    handler.close()


def test_rotating_handler_defaults(log_path: pathlib.Path) -> None:
    """Constructing with defaults should disable rotation thresholds."""

    handler = FemtoRotatingFileHandler(str(log_path))
    try:
        assert handler.max_bytes == 0, "defaults must disable rollover"
        assert handler.backup_count == 0, "defaults must disable backups"
    finally:
        _close_handler(handler)


def test_rotating_handler_accepts_options(log_path: pathlib.Path) -> None:
    """Supplying HandlerOptions should configure queue behaviour."""

    options = HandlerOptions(capacity=32, flush_interval=2, policy="block")
    handler = FemtoRotatingFileHandler(
        str(log_path), max_bytes=1024, backup_count=3, options=options
    )
    try:
        assert handler.max_bytes == 1024, "max_bytes setter must persist"
        assert handler.backup_count == 3, "backup_count setter must persist"
    finally:
        _close_handler(handler)


@pytest.mark.parametrize(
    ("max_bytes", "backup_count"),
    [(1024, 0), (0, 3)],
)
def test_rotating_handler_rejects_partial_thresholds(
    log_path: pathlib.Path, max_bytes: int, backup_count: int
) -> None:
    """Partial rotation thresholds should raise a clear error."""

    message = (
        "both max_bytes and backup_count must be > 0 to enable rotation; "
        "set both to 0 to disable"
    )
    with pytest.raises(ValueError, match=message):
        FemtoRotatingFileHandler(
            str(log_path), max_bytes=max_bytes, backup_count=backup_count
        )
