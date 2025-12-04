"""Behavioural tests covering rotating file handler size-based rotation."""

from __future__ import annotations

"""BDD steps validating rotating file handler rollover behaviour."""

from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    FemtoRotatingFileHandler,
    HandlerOptions,
    _clear_rotating_fresh_failure_for_test,
    _force_rotating_fresh_failure_for_test,
)

if TYPE_CHECKING:
    import pathlib
    from collections.abc import Callable

    from syrupy.assertion import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "rotating_handler_rotation.feature"))


@dataclass
class RotatingContext:
    """Hold rotating handler state and log file path for a scenario."""

    handler: FemtoRotatingFileHandler
    path: pathlib.Path
    closed: bool = False


@pytest.fixture
def rotating_context_factory(
    tmp_path: pathlib.Path, request: pytest.FixtureRequest
) -> Callable[[int, int], RotatingContext]:
    contexts: list[RotatingContext] = []

    def _finalise() -> None:
        for ctx in contexts:
            if not ctx.closed:
                ctx.handler.close()
                ctx.closed = True

    request.addfinalizer(_finalise)

    def _build(max_bytes: int, backup_count: int) -> RotatingContext:
        path = tmp_path / "rotating.log"
        handler = FemtoRotatingFileHandler(
            str(path),
            options=HandlerOptions(rotation=(max_bytes, backup_count)),
        )
        ctx = RotatingContext(handler=handler, path=path)
        contexts.append(ctx)
        return ctx

    return _build


@pytest.fixture
def force_rotating_failure(
    request: pytest.FixtureRequest,
) -> Callable[[int, str], None]:
    registered = False

    def _activate(count: int, reason: str) -> None:
        nonlocal registered
        _force_rotating_fresh_failure_for_test(count, reason)
        if not registered:
            request.addfinalizer(_clear_rotating_fresh_failure_for_test)
            registered = True

    return _activate


@given(
    parsers.parse(
        "a rotating handler with max bytes {max_bytes:d} and backup count {backup_count:d}"
    ),
    target_fixture="rotating_ctx",
)
def given_rotating_handler(
    rotating_context_factory: Callable[[int, int], RotatingContext],
    max_bytes: int,
    backup_count: int,
) -> RotatingContext:
    return rotating_context_factory(max_bytes, backup_count)


@given(
    parsers.parse(
        "a rotating handler forcing reopen failure with max bytes {max_bytes:d} and backup count {backup_count:d}"
    ),
    target_fixture="rotating_ctx",
)
def given_rotating_handler_forcing_reopen_failure(
    rotating_context_factory: Callable[[int, int], RotatingContext],
    max_bytes: int,
    backup_count: int,
    force_rotating_failure: Callable[[int, str], None],
) -> RotatingContext:
    force_rotating_failure(1, "python scenario")
    return rotating_context_factory(max_bytes, backup_count)


@when(
    parsers.parse('I log record "{message}" at level "{level}" for logger "{logger}"')
)
def when_log_record(
    rotating_ctx: RotatingContext, message: str, level: str, logger: str
) -> None:
    rotating_ctx.handler.handle(logger, level, message)


@when("I close the rotating handler")
def when_close_handler(rotating_ctx: RotatingContext) -> None:
    rotating_ctx.handler.close()
    rotating_ctx.closed = True


@then("the rotating log files match snapshot")
def then_log_files_snapshot(
    rotating_ctx: RotatingContext, snapshot: SnapshotAssertion
) -> None:
    if not rotating_ctx.closed:
        rotating_ctx.handler.close()
        rotating_ctx.closed = True

    base = rotating_ctx.path
    contents = {
        candidate.name: candidate.read_text().rstrip("\n")
        for candidate in sorted(base.parent.glob(f"{base.name}*"))
    }
    snapshot.assert_match(contents)
