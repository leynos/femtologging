"""Behavioural tests covering rotating file handler size-based rotation."""

from __future__ import annotations

from dataclasses import dataclass
import pathlib

import pytest
from pytest_bdd import given, parsers, scenarios, then, when
from syrupy.assertion import SnapshotAssertion

from femtologging import FemtoRotatingFileHandler, HandlerOptions


scenarios("features/rotating_handler_rotation.feature")


@dataclass
class RotatingContext:
    handler: FemtoRotatingFileHandler
    path: pathlib.Path
    closed: bool = False


@given(
    parsers.parse(
        "a rotating handler with max bytes {max_bytes:d} and backup count {backup_count:d}"
    ),
    target_fixture="rotating_ctx",
)
def given_rotating_handler(
    tmp_path: pathlib.Path,
    max_bytes: int,
    backup_count: int,
    request: pytest.FixtureRequest,
) -> RotatingContext:
    path = tmp_path / "rotating.log"
    handler = FemtoRotatingFileHandler(
        str(path),
        options=HandlerOptions(rotation=(max_bytes, backup_count)),
    )
    ctx = RotatingContext(handler=handler, path=path)

    def _finaliser() -> None:
        if not ctx.closed:
            ctx.handler.close()
            ctx.closed = True

    request.addfinalizer(_finaliser)
    return ctx


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
    contents = {}
    for candidate in sorted(base.parent.glob(f"{base.name}*")):
        contents[candidate.name] = candidate.read_text()
    assert contents == snapshot
