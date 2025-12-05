"""Tests for FemtoRotatingFileHandler Python bindings and rotation thresholds."""

from __future__ import annotations

import re
import typing as typ
from contextlib import contextmanager

import pytest

from femtologging import (
    ROTATION_VALIDATION_MSG,
    FemtoRotatingFileHandler,
    HandlerOptions,
)

if typ.TYPE_CHECKING:
    import pathlib


@pytest.fixture(name="log_path")
def fixture_log_path(tmp_path: pathlib.Path) -> pathlib.Path:
    """Provide a unique log file path for rotating handler tests."""
    return tmp_path / "rotating.log"


@contextmanager
def rotating_handler(
    path: str,
    *,
    max_bytes: int = 0,
    backup_count: int = 0,
    options: HandlerOptions | None = None,
) -> typ.Iterator[FemtoRotatingFileHandler]:
    """Context manager for rotating handler lifecycle."""
    derived_options = options
    if derived_options is None:
        derived_options = HandlerOptions(rotation=(max_bytes, backup_count))
    elif max_bytes or backup_count:
        msg = "rotating_handler options already provided; do not pass rotation"
        raise ValueError(msg)

    handler = FemtoRotatingFileHandler(path, options=derived_options)
    try:
        yield handler
    finally:
        handler.close()


def test_rotating_handler_defaults(log_path: pathlib.Path) -> None:
    """Constructing with defaults should disable rotation thresholds."""
    with rotating_handler(str(log_path)) as handler:
        assert handler.max_bytes == 0, "defaults must disable rollover"
        assert handler.backup_count == 0, "defaults must disable backups"


def test_rotating_handler_invalid_policy(log_path: pathlib.Path) -> None:
    """Supplying an invalid policy value should raise an error."""
    invalid_policy_value = typ.cast(
        "typ.Any",
        "invalid_policy",
    )  # Exercise runtime validation with a value rejected at type-check time.
    invalid_options = HandlerOptions(
        capacity=32,
        flush_interval=2,
        policy=invalid_policy_value,
        rotation=(1024, 3),
    )
    with (
        pytest.raises(
            ValueError,
            match=(
                r"invalid overflow policy: '.*'\. Valid options are: "
                r"drop, block, timeout:N"
            ),
        ),
        rotating_handler(str(log_path), options=invalid_options),
    ):
        pass


def test_rotating_handler_missing_policy(log_path: pathlib.Path) -> None:
    """Omitting policy should use defaults and preserve rotation settings."""
    options = HandlerOptions(
        capacity=32,
        flush_interval=2,
        rotation=(1024, 3),
    )

    with rotating_handler(str(log_path), options=options) as handler:
        assert handler.max_bytes == 1024, "rotation max_bytes should still apply"
        assert handler.backup_count == 3, "rotation backup_count should still apply"


def test_rotating_handler_accepts_options(log_path: pathlib.Path) -> None:
    """Supplying HandlerOptions should configure queue behaviour."""
    options = HandlerOptions(
        capacity=32,
        flush_interval=2,
        policy="block",
        rotation=(1024, 3),
    )
    with rotating_handler(str(log_path), options=options) as handler:
        assert handler.max_bytes == 1024, "max_bytes setter must persist"
        assert handler.backup_count == 3, "backup_count setter must persist"
        handler.handle("rotating", "INFO", "probe message")
        assert isinstance(handler.flush(), bool), "flush must return a boolean"


@pytest.mark.parametrize(
    ("max_bytes", "backup_count", "should_error"),
    [
        (1024, 0, True),
        (512, 0, True),
        (0, 3, True),
        (0, 1, True),
        (0, 0, False),
    ],
)
def test_rotating_handler_threshold_validation(
    log_path: pathlib.Path,
    max_bytes: int,
    backup_count: int,
    should_error: bool,  # noqa: FBT001
) -> None:
    """Rotation thresholds must be paired or omitted entirely."""
    if should_error:
        with pytest.raises(ValueError, match=re.escape(ROTATION_VALIDATION_MSG)):
            FemtoRotatingFileHandler(
                str(log_path),
                options=HandlerOptions(rotation=(max_bytes, backup_count)),
            )
    else:
        with rotating_handler(
            str(log_path),
            options=HandlerOptions(rotation=(max_bytes, backup_count)),
        ):
            pass
