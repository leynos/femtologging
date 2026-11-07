"""Behavioural tests covering timed rotating file handler rollover."""

from __future__ import annotations

import dataclasses
import datetime as dt
import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    FemtoTimedRotatingFileHandler,
    TimedHandlerOptions,
    _clear_timed_rotation_test_times_for_test,
    _has_test_util,
    _set_timed_rotation_test_times_for_test,
)

pytestmark = pytest.mark.skipif(
    not _has_test_util,
    reason="requires Rust extension built with the 'test-util' feature",
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy.assertion import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "timed_rotating_handler_rotation.feature"))


@dataclasses.dataclass(slots=True)
class TimedRotatingContext:
    """Hold timed rotating handler state and output path for a scenario."""

    handler: FemtoTimedRotatingFileHandler
    path: Path
    spec: TimedRotationSpec
    closed: bool = False


@dataclasses.dataclass(frozen=True, slots=True)
class TimedRotationSpec:
    """Parameter bundle for timed rotating handler construction."""

    when: str
    interval: int
    backup_count: int
    use_utc: bool
    at_time: dt.time | None = None


@pytest.fixture
def timed_rotating_context_factory(
    tmp_path: Path,
) -> cabc.Iterator[cabc.Callable[[TimedRotationSpec], TimedRotatingContext]]:
    """Yield a timed handler builder and close its handlers at teardown."""
    contexts: list[TimedRotatingContext] = []

    def _build(spec: TimedRotationSpec) -> TimedRotatingContext:
        """Create a timed rotating handler from spec and track it for teardown."""
        path = tmp_path / "timed.log"
        options = TimedHandlerOptions(
            when=spec.when,
            interval=spec.interval,
            backup_count=spec.backup_count,
            utc=spec.use_utc,
            at_time=spec.at_time,
        )
        handler = FemtoTimedRotatingFileHandler(str(path), options=options)
        ctx = TimedRotatingContext(handler=handler, path=path, spec=spec)
        contexts.append(ctx)
        return ctx

    yield _build

    for ctx in contexts:
        if not ctx.closed:
            ctx.handler.close()
            ctx.closed = True
    _clear_timed_rotation_test_times_for_test()


def _parse_time(value: str) -> dt.time:
    """Parse an HH:MM:SS string into a time of day."""
    hours, minutes, seconds = [int(part) for part in value.split(":")]
    return dt.time(hours, minutes, seconds)


def _epoch_millis(value: str) -> int:
    """Convert an ISO-8601 timestamp to epoch milliseconds, assuming UTC if naive."""
    timestamp = dt.datetime.fromisoformat(value)
    if timestamp.tzinfo is None:
        timestamp = timestamp.replace(tzinfo=dt.UTC)
    return int(timestamp.timestamp() * 1000)


@given(
    parsers.parse(
        'a timed rotating handler with when "{when_value}" interval {interval:d} '
        "backup count {backup_count:d} utc enabled"
    ),
    target_fixture="timed_ctx",
)
def given_timed_rotating_handler(
    timed_rotating_context_factory: cabc.Callable[
        [TimedRotationSpec], TimedRotatingContext
    ],
    when_value: str,
    interval: int,
    backup_count: int,
) -> TimedRotatingContext:
    """Build a UTC timed rotating handler with the given rotation settings."""
    return timed_rotating_context_factory(
        TimedRotationSpec(
            when=when_value,
            interval=interval,
            backup_count=backup_count,
            use_utc=True,
        )
    )


@given("timed rotation test times:", target_fixture="timed_rotation_test_times")
def given_timed_rotation_test_times(datatable: list[list[str]]) -> list[int]:
    """Set the test clock from datatable timestamps and return them."""
    timestamps = [_epoch_millis(row[0]) for row in datatable[1:]]
    _set_timed_rotation_test_times_for_test(timestamps)
    return timestamps


@given(
    parsers.parse('the handler at_time is "{time_value}"'),
    target_fixture="timed_ctx",
)
def given_handler_at_time(
    timed_rotating_context_factory: cabc.Callable[
        [TimedRotationSpec], TimedRotatingContext
    ],
    timed_ctx: TimedRotatingContext,
    time_value: str,
) -> TimedRotatingContext:
    """Close the current timed handler and rebuild it with the given at_time."""
    timed_ctx.handler.close()
    timed_ctx.closed = True
    return timed_rotating_context_factory(
        dataclasses.replace(timed_ctx.spec, at_time=_parse_time(time_value))
    )


@when(
    parsers.parse(
        'I log timed record "{message}" at level "{level}" for logger "{logger}"'
    )
)
def when_log_timed_record(
    timed_ctx: TimedRotatingContext, message: str, level: str, logger: str
) -> None:
    """Send a log record to the timed handler with the given level and logger."""
    timed_ctx.handler.handle(logger, level, message)


@when("I close the timed rotating handler")
def when_close_timed_handler(timed_ctx: TimedRotatingContext) -> None:
    """Close the timed handler and mark the context as closed."""
    timed_ctx.handler.close()
    timed_ctx.closed = True


@then("the timed rotating log files match snapshot")
def then_timed_log_files_snapshot(
    timed_ctx: TimedRotatingContext, snapshot: SnapshotAssertion
) -> None:
    """Close the handler if open, then snapshot the rotated log file contents."""
    if not timed_ctx.closed:
        timed_ctx.handler.close()
        timed_ctx.closed = True

    base = timed_ctx.path
    contents = {
        candidate.name: candidate.read_text().rstrip("\n")
        for candidate in sorted(base.parent.glob(f"{base.name}*"))
    }
    snapshot.assert_match(contents)
