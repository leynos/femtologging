"""Behavioural tests covering rotating file handler size-based rotation."""

from __future__ import annotations

import dataclasses
import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    FemtoRotatingFileHandler,
    HandlerOptions,
    _clear_rotating_fresh_failure_for_test,
    _force_rotating_fresh_failure_for_test,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy.assertion import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "rotating_handler_rotation.feature"))


@dataclasses.dataclass(slots=True)
class RotatingContext:
    """Hold rotating handler state and log file path for a scenario."""

    handler: FemtoRotatingFileHandler
    path: Path
    closed: bool = False


@pytest.fixture
def rotating_context_factory(
    tmp_path: Path,
) -> cabc.Iterator[cabc.Callable[[int, int], RotatingContext]]:
    """Yield a rotating handler builder and close its handlers at teardown."""
    contexts: list[RotatingContext] = []

    def _build(max_bytes: int, backup_count: int) -> RotatingContext:
        """Create a size-rotating file handler and track it for teardown."""
        path = tmp_path / "rotating.log"
        handler = FemtoRotatingFileHandler(
            str(path),
            options=HandlerOptions(rotation=(max_bytes, backup_count)),
        )
        ctx = RotatingContext(handler=handler, path=path)
        contexts.append(ctx)
        return ctx

    yield _build

    for ctx in contexts:
        if not ctx.closed:
            ctx.handler.close()
            ctx.closed = True


@pytest.fixture
def force_rotating_failure(
    request: pytest.FixtureRequest,
) -> cabc.Callable[[int, str], None]:
    """Return a helper that arms a forced reopen failure, cleared after the test."""
    registered = False

    def _activate(count: int, reason: str) -> None:
        """Arm the forced failure and register its cleanup on first use."""
        nonlocal registered
        _force_rotating_fresh_failure_for_test(count, reason)
        if not registered:
            request.addfinalizer(_clear_rotating_fresh_failure_for_test)
            registered = True

    return _activate


@given(
    parsers.parse(
        "a rotating handler with max bytes {max_bytes:d} and backup count "
        "{backup_count:d}"
    ),
    target_fixture="rotating_ctx",
)
def given_rotating_handler(
    rotating_context_factory: cabc.Callable[[int, int], RotatingContext],
    max_bytes: int,
    backup_count: int,
) -> RotatingContext:
    """Build a rotating handler with the given max bytes and backup count."""
    return rotating_context_factory(max_bytes, backup_count)


@given(
    parsers.parse(
        "a rotating handler forcing reopen failure with max bytes {max_bytes:d} and "
        "backup count {backup_count:d}"
    ),
    target_fixture="rotating_ctx",
)
def given_rotating_handler_forcing_reopen_failure(
    rotating_context_factory: cabc.Callable[[int, int], RotatingContext],
    max_bytes: int,
    backup_count: int,
    force_rotating_failure: cabc.Callable[[int, str], None],
) -> RotatingContext:
    """Arm one forced reopen failure, then build a rotating handler."""
    force_rotating_failure(1, "python scenario")
    return rotating_context_factory(max_bytes, backup_count)


@when(
    parsers.parse('I log record "{message}" at level "{level}" for logger "{logger}"')
)
def when_log_record(
    rotating_ctx: RotatingContext, message: str, level: str, logger: str
) -> None:
    """Send a record with the given level and logger to the rotating handler."""
    rotating_ctx.handler.handle(logger, level, message)


@when("I close the rotating handler")
def when_close_handler(rotating_ctx: RotatingContext) -> None:
    """Close the rotating handler and mark the context as closed."""
    rotating_ctx.handler.close()
    rotating_ctx.closed = True


@then("the rotating log files match snapshot")
def then_log_files_snapshot(
    rotating_ctx: RotatingContext, snapshot: SnapshotAssertion
) -> None:
    """Close the handler if open, then snapshot the rotated log file contents."""
    if not rotating_ctx.closed:
        rotating_ctx.handler.close()
        rotating_ctx.closed = True

    base = rotating_ctx.path
    contents = {
        candidate.name: candidate.read_text().rstrip("\n")
        for candidate in sorted(base.parent.glob(f"{base.name}*"))
    }
    snapshot.assert_match(contents)
