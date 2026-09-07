"""BDD steps for file, rotating, and timed rotating handler builders.

Star-imported by ``tests/steps/test_handler_builders_steps.py``; see
``tests/steps/handler_builders_support.py`` for the rationale.
"""

from __future__ import annotations

import re
import typing as typ

import pytest
from pytest_bdd import given, parsers, then, when

import femtologging.config as config_module
from femtologging import (
    FileHandlerBuilder,
    HandlerConfigError,
    OverflowPolicy,
    RotatingFileHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
)
from tests.steps.handler_builders_support import (
    FileBuilder,
    normalize_builder_path,
    parse_clock_time,
    require_rotating_builder,
    require_timed_builder,
)

if typ.TYPE_CHECKING:
    from pathlib import Path

    from syrupy import SnapshotAssertion


@given('a FileHandlerBuilder for path "test.log"', target_fixture="file_builder")
def given_file_builder(tmp_path: Path) -> FileHandlerBuilder:
    """Create a file-handler builder rooted at the scenario's temporary path."""
    return FileHandlerBuilder(str(tmp_path / "test.log"))


@given(
    'a RotatingFileHandlerBuilder for path "test.log"', target_fixture="file_builder"
)
def given_rotating_file_builder(tmp_path: Path) -> RotatingFileHandlerBuilder:
    """Create a rotating file-handler builder for the scenario's temporary path."""
    return RotatingFileHandlerBuilder(str(tmp_path / "test.log"))


@given(
    'a TimedRotatingFileHandlerBuilder for path "test.log"',
    target_fixture="file_builder",
)
def given_timed_rotating_file_builder(
    tmp_path: Path,
) -> TimedRotatingFileHandlerBuilder:
    """Create a timed rotating builder for the scenario's temporary path."""
    return TimedRotatingFileHandlerBuilder(str(tmp_path / "test.log"))


def _builder_from_dict_config(class_path: str, path: Path) -> FileBuilder:
    """Build a handler builder through the dictConfig conversion helper.

    The helper is internal but provides the only builder-returning path that
    mirrors the Python logging schema without mutating global configuration.

    Returns
    -------
    FileBuilder
        The builder ``dictConfig`` would use for *class_path*.
    """
    return typ.cast(
        "FileBuilder",
        config_module._build_handler_from_dict(
            "h",
            {"class": class_path, "args": [str(path)]},
        ),
    )


@given(
    'a dictConfig RotatingFileHandlerBuilder for path "test.log"',
    target_fixture="file_builder",
)
def given_dictconfig_rotating_file_builder(
    tmp_path: Path,
) -> RotatingFileHandlerBuilder:
    """Create a rotating builder through the ``dictConfig`` conversion path."""
    builder = _builder_from_dict_config(
        "logging.handlers.RotatingFileHandler", tmp_path / "test.log"
    )
    return require_rotating_builder(builder)


@given(
    'a dictConfig TimedRotatingFileHandlerBuilder for path "test.log"',
    target_fixture="file_builder",
)
def given_dictconfig_timed_rotating_file_builder(
    tmp_path: Path,
) -> TimedRotatingFileHandlerBuilder:
    """Create a timed rotating builder through the ``dictConfig`` path."""
    builder = _builder_from_dict_config(
        "logging.handlers.TimedRotatingFileHandler", tmp_path / "test.log"
    )
    return require_timed_builder(builder)


@when(parsers.parse("I set file capacity {capacity:d}"))
def when_set_file_capacity(file_builder: FileBuilder, capacity: int) -> FileBuilder:
    """Apply the requested capacity to the current file-handler builder."""
    return file_builder.with_capacity(capacity)


@when(parsers.parse("I set flush after records {interval:d}"))
def when_set_flush_after_records(
    file_builder: FileBuilder, interval: int
) -> FileBuilder:
    """Set how many records the current builder accepts before flushing."""
    return file_builder.with_flush_after_records(interval)


@when("I set overflow policy to timeout with 500ms")
def when_set_overflow_policy_timeout(file_builder: FileBuilder) -> FileBuilder:
    """Configure the fixed 500-millisecond timeout overflow policy."""
    return file_builder.with_overflow_policy(OverflowPolicy.timeout(500))


@when(parsers.parse('I set file formatter "{formatter_id}"'))
def when_set_file_formatter(
    file_builder: FileBuilder, formatter_id: str
) -> FileBuilder:
    """Set the formatter identifier on the current file-handler builder."""
    return file_builder.with_formatter(formatter_id)


@when(parsers.parse("I set max bytes {max_bytes:d}"))
def when_set_max_bytes(file_builder: FileBuilder, max_bytes: int) -> FileBuilder:
    """Set the maximum size of a rotating file before rollover."""
    return require_rotating_builder(file_builder).with_max_bytes(max_bytes)


@when(parsers.parse("I set backup count {backup_count:d}"))
def when_set_backup_count(file_builder: FileBuilder, backup_count: int) -> FileBuilder:
    """Set the number of backup files retained by the rotating builder."""
    return require_rotating_builder(file_builder).with_backup_count(backup_count)


@when(parsers.parse('I set timed rotation when "{when_value}"'))
def when_set_timed_rotation_when(
    file_builder: FileBuilder, when_value: str
) -> FileBuilder:
    """Set the time unit used by the timed rotating builder."""
    return require_timed_builder(file_builder).with_when(when_value)


@when(parsers.parse("I set timed rotation interval {interval:d}"))
def when_set_timed_rotation_interval(
    file_builder: FileBuilder, interval: int
) -> FileBuilder:
    """Set the timed rotation interval on the current builder."""
    return require_timed_builder(file_builder).with_interval(interval)


@when(parsers.parse("I set timed rotation backup count {backup_count:d}"))
def when_set_timed_rotation_backup_count(
    file_builder: FileBuilder, backup_count: int
) -> FileBuilder:
    """Set the number of timed-rotation backups retained by the builder."""
    return require_timed_builder(file_builder).with_backup_count(backup_count)


@when("I enable timed rotation UTC mode")
def when_enable_timed_rotation_utc(file_builder: FileBuilder) -> FileBuilder:
    """Enable UTC timestamps for timed rotation."""
    use_utc = True
    return require_timed_builder(file_builder).with_utc(use_utc)


@when(parsers.parse('I set timed rotation at time "{time_value}"'))
def when_set_timed_rotation_at_time(
    file_builder: FileBuilder, time_value: str
) -> FileBuilder:
    """Set the parsed clock time at which timed rotation occurs."""
    timed = require_timed_builder(file_builder)
    return timed.with_at_time(parse_clock_time(time_value))


@then("the file handler builder matches snapshot")
def then_file_builder_snapshot(
    file_builder: FileHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    """Compare the file builder mapping with its syrupy snapshot."""
    data = normalize_builder_path(file_builder.as_dict())
    assert data == snapshot, "file builder dict must match snapshot"
    file_builder.build().close()


@then("the rotating file handler builder matches snapshot")
def then_rotating_file_builder_snapshot(
    file_builder: FileBuilder, snapshot: SnapshotAssertion
) -> None:
    """Compare the rotating builder mapping with its syrupy snapshot."""
    rotating = require_rotating_builder(file_builder)
    data = normalize_builder_path(rotating.as_dict())
    assert data == snapshot, "rotating file builder dict must match snapshot"
    rotating.build().close()


@then("the timed rotating file handler builder matches snapshot")
def then_timed_rotating_file_builder_snapshot(
    file_builder: FileBuilder, snapshot: SnapshotAssertion
) -> None:
    """Compare the timed rotating mapping with its syrupy snapshot."""
    timed = require_timed_builder(file_builder)
    data = normalize_builder_path(timed.as_dict())
    assert data == snapshot, "timed rotating builder dict must match snapshot"
    timed.build().close()


@then("the file handler builder with timeout overflow matches snapshot")
def then_file_builder_timeout_snapshot(
    file_builder: FileHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    """Verify timeout fields and compare the file builder with its snapshot."""
    data = normalize_builder_path(file_builder.as_dict())
    assert data["overflow_policy"] == "timeout", "must record timeout policy"
    assert data["timeout_ms"] == 500, "must record configured timeout"
    assert data == snapshot, "snapshot must include timeout fields"
    file_builder.build().close()


@then("building the file handler fails")
def then_file_builder_fails(file_builder: FileHandlerBuilder) -> None:
    """Verify that building the current file handler raises configuration error."""
    with pytest.raises(HandlerConfigError):
        file_builder.build()


@then(parsers.parse('building the rotating file handler fails with "{message}"'))
def then_rotating_file_builder_fails(file_builder: FileBuilder, message: str) -> None:
    """Verify that the rotating builder raises the expected configuration error."""
    rotating = require_rotating_builder(file_builder)
    with pytest.raises(HandlerConfigError, match=re.escape(message)):
        rotating.build()


@then(
    parsers.parse('setting rotating file capacity {capacity:d} fails with "{message}"')
)
def then_setting_rotating_capacity_fails(
    file_builder: FileBuilder, capacity: int, message: str
) -> None:
    """Verify invalid rotating capacity raises ValueError."""
    rotating = require_rotating_builder(file_builder)
    with pytest.raises(ValueError, match=re.escape(message)):
        rotating.with_capacity(capacity)


@then(parsers.parse('setting max bytes {max_bytes:d} fails with "{message}"'))
def then_setting_max_bytes_fails(
    file_builder: FileBuilder, max_bytes: int, message: str
) -> None:
    """Verify that an invalid rotating maximum size raises the expected error."""
    rotating = require_rotating_builder(file_builder)
    with pytest.raises(ValueError, match=re.escape(message)):
        rotating.with_max_bytes(max_bytes)


@then(parsers.parse('setting backup count {backup_count:d} fails with "{message}"'))
def then_setting_backup_count_fails(
    file_builder: FileBuilder, backup_count: int, message: str
) -> None:
    """Verify that an invalid rotating backup count raises the expected error."""
    rotating = require_rotating_builder(file_builder)
    with pytest.raises(ValueError, match=re.escape(message)):
        rotating.with_backup_count(backup_count)


@then(
    parsers.parse('setting timed rotation when "{when_value}" fails with "{message}"')
)
def then_setting_timed_rotation_when_fails(
    file_builder: FileBuilder, when_value: str, message: str
) -> None:
    """Verify that an invalid timed rotation unit raises the expected error."""
    timed = require_timed_builder(file_builder)
    with pytest.raises(ValueError, match=re.escape(message)):
        timed.with_when(when_value)


@then(
    parsers.parse(
        'setting timed rotation at time "{time_value}" fails with "{message}"'
    )
)
def then_setting_timed_rotation_at_time_fails(
    file_builder: FileBuilder, time_value: str, message: str
) -> None:
    """Verify that an invalid timed rotation clock value raises the expected error."""
    timed = require_timed_builder(file_builder)
    with pytest.raises(ValueError, match=re.escape(message)):
        timed.with_at_time(parse_clock_time(time_value))


@then(parsers.parse('setting zero rotation thresholds fails with "{message}"'))
def then_zero_rotation_thresholds_fail(file_builder: FileBuilder, message: str) -> None:
    """Verify that zero rotating thresholds fail during handler construction."""
    rotating = require_rotating_builder(file_builder)
    with pytest.raises(HandlerConfigError, match=re.escape(message)):
        rotating.with_max_bytes(0).with_backup_count(0).build()


@then(parsers.parse("setting flush after records {interval:d} fails"))
def then_setting_flush_after_records_fails(
    file_builder: FileBuilder, interval: int
) -> None:
    """Verify that an invalid flush interval raises its expected exception."""
    exc = ValueError if interval == 0 else OverflowError
    with pytest.raises(exc):
        file_builder.with_flush_after_records(interval)
