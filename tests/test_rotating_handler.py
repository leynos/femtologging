"""Tests for FemtoRotatingFileHandler Python bindings and rotation thresholds."""

from __future__ import annotations

import re
import typing as typ
from contextlib import closing

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from femtologging import (
    ROTATION_VALIDATION_MSG,
    FemtoRotatingFileHandler,
    HandlerOptions,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    import pathlib

    type RotatingHandlerFactory = cabc.Callable[
        [HandlerOptions | None], FemtoRotatingFileHandler
    ]


def _is_paired(max_bytes: int, backup_count: int) -> bool:
    """Return whether the rotation thresholds are both set or both omitted."""
    return (max_bytes > 0) == (backup_count > 0)


@pytest.fixture(name="log_path")
def fixture_log_path(tmp_path: pathlib.Path) -> pathlib.Path:
    """Provide a unique log file path for rotating handler tests."""
    return tmp_path / "rotating.log"


@pytest.fixture(name="open_rotating_handler")
def fixture_open_rotating_handler(
    log_path: pathlib.Path,
) -> cabc.Iterator[RotatingHandlerFactory]:
    """Open rotating handlers on the test log path and close them afterwards."""
    handlers: list[FemtoRotatingFileHandler] = []

    def open_handler(
        options: HandlerOptions | None = None,
    ) -> FemtoRotatingFileHandler:
        handler = FemtoRotatingFileHandler(
            str(log_path),
            options=options if options is not None else HandlerOptions(),
        )
        handlers.append(handler)
        return handler

    try:
        yield open_handler
    finally:
        for handler in handlers:
            handler.close()


def test_rotating_handler_defaults(
    open_rotating_handler: RotatingHandlerFactory,
) -> None:
    """Constructing with defaults should disable rotation thresholds."""
    handler = open_rotating_handler(None)

    assert handler.max_bytes == 0, "defaults must disable rollover"
    assert handler.backup_count == 0, "defaults must disable backups"


def test_rotating_handler_invalid_policy() -> None:
    """Supplying an invalid policy value should raise an error at construction."""
    # Policy validation now occurs in HandlerOptions constructor (not handler).
    invalid_policy_value = typ.cast(
        "typ.Any",
        "invalid_policy",
    )  # Exercise runtime validation with a value rejected at type-check time.
    with pytest.raises(
        ValueError,
        match=(
            r"invalid overflow policy: '.*'\. Valid options are: "
            r"drop, block, timeout:N"
        ),
    ):
        HandlerOptions(
            capacity=32,
            flush_interval=2,
            policy=invalid_policy_value,
            rotation=(1024, 3),
        )


def test_rotating_handler_missing_policy(
    open_rotating_handler: RotatingHandlerFactory,
) -> None:
    """Omitting policy should use defaults and preserve rotation settings."""
    handler = open_rotating_handler(
        HandlerOptions(capacity=32, flush_interval=2, rotation=(1024, 3))
    )

    assert handler.max_bytes == 1024, "rotation max_bytes should still apply"
    assert handler.backup_count == 3, "rotation backup_count should still apply"


def test_rotating_handler_accepts_options(
    open_rotating_handler: RotatingHandlerFactory,
) -> None:
    """Supplying HandlerOptions should configure queue behaviour."""
    handler = open_rotating_handler(
        HandlerOptions(
            capacity=32,
            flush_interval=2,
            policy="block",
            rotation=(1024, 3),
        )
    )

    assert handler.max_bytes == 1024, "max_bytes setter must persist"
    assert handler.backup_count == 3, "backup_count setter must persist"
    handler.handle("rotating", "INFO", "probe message")
    assert isinstance(handler.flush(), bool), "flush must return a boolean"


@pytest.mark.parametrize(
    ("max_bytes", "backup_count"),
    [
        pytest.param(1024, 0, id="size-without-backups"),
        pytest.param(512, 0, id="small-size-without-backups"),
        pytest.param(0, 3, id="backups-without-size"),
        pytest.param(0, 1, id="single-backup-without-size"),
    ],
)
def test_rotating_handler_rejects_unpaired_thresholds(
    log_path: pathlib.Path,
    max_bytes: int,
    backup_count: int,
) -> None:
    """Setting only one rotation threshold must fail fast."""
    with pytest.raises(ValueError, match=re.escape(ROTATION_VALIDATION_MSG)):
        FemtoRotatingFileHandler(
            str(log_path),
            options=HandlerOptions(rotation=(max_bytes, backup_count)),
        )


@pytest.mark.parametrize(
    ("max_bytes", "backup_count"),
    [
        pytest.param(0, 0, id="rotation-disabled"),
        pytest.param(1024, 3, id="rotation-enabled"),
    ],
)
def test_rotating_handler_accepts_paired_thresholds(
    open_rotating_handler: RotatingHandlerFactory,
    max_bytes: int,
    backup_count: int,
) -> None:
    """Rotation thresholds that are both set or both omitted are accepted."""
    handler = open_rotating_handler(HandlerOptions(rotation=(max_bytes, backup_count)))

    assert (handler.max_bytes, handler.backup_count) == (max_bytes, backup_count), (
        "paired rotation thresholds must round-trip unchanged, but "
        f"({max_bytes}, {backup_count}) became "
        f"({handler.max_bytes}, {handler.backup_count})"
    )


# Handler construction touches the filesystem and spawns a worker thread, so
# this property samples the threshold space sparsely; the named cases above
# remain the regression record.
@settings(max_examples=25, deadline=None)
@given(
    max_bytes=st.integers(min_value=0, max_value=1 << 20),
    backup_count=st.integers(min_value=0, max_value=16),
)
def test_rotating_handler_threshold_pairing_is_the_only_rule(
    tmp_path_factory: pytest.TempPathFactory,
    max_bytes: int,
    backup_count: int,
) -> None:
    """Validation depends only on whether both thresholds are non-zero."""
    path = tmp_path_factory.mktemp("rotating") / "rotating.log"

    if _is_paired(max_bytes, backup_count):
        options = HandlerOptions(rotation=(max_bytes, backup_count))
        with closing(FemtoRotatingFileHandler(str(path), options=options)) as handler:
            assert (handler.max_bytes, handler.backup_count) == (
                max_bytes,
                backup_count,
            ), (
                f"paired thresholds ({max_bytes}, {backup_count}) must be "
                "accepted verbatim, but the handler reported "
                f"({handler.max_bytes}, {handler.backup_count})"
            )
    else:
        with pytest.raises(ValueError, match=re.escape(ROTATION_VALIDATION_MSG)):
            FemtoRotatingFileHandler(
                str(path),
                options=HandlerOptions(rotation=(max_bytes, backup_count)),
            )
