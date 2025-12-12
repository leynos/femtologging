"""BDD steps for Rust log crate compatibility."""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    FemtoStreamHandler,
    StreamHandlerBuilder,
    get_logger,
    rust,
    setup_rust_logging,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy.assertion import SnapshotAssertion

    Iterator = cabc.Iterator
    Sequence = cabc.Sequence

FEATURES = Path(__file__).resolve().parents[1] / "features"

pytestmark = [pytest.mark.log_compat]

scenarios(str(FEATURES / "rust_log_compat.feature"))


@given(
    parsers.parse('a stream handler attached to logger "{name}"'),
    target_fixture="handler_ctx",
)
def given_stream_handler(name: str) -> Iterator[tuple[FemtoStreamHandler, str]]:
    """Attach a stderr stream handler to the named logger."""
    handler = StreamHandlerBuilder.stderr().build()
    logger = get_logger(name)
    logger.add_handler(handler)
    try:
        yield handler, name
    finally:
        logger.clear_handlers()
        handler.close()


@when("I set up rust logging bridge")
def when_setup_bridge() -> None:
    """Install the Rust `log` crate adapter."""
    setup_rust_logging()


@when(parsers.parse('I set logger "{name}" level to "{level}"'))
def when_set_logger_level(name: str, level: str) -> None:
    """Change the named logger's minimum level for the scenario."""
    logger = get_logger(name)
    logger.set_level(level)


@when(
    parsers.parse('I emit a Rust log "{message}" at "{level}" with target "{target}"'),
    target_fixture="output",
)
def when_emit_rust_log(
    handler_ctx: tuple[FemtoStreamHandler, str],
    capfd: pytest.CaptureFixture[str],
    *,
    message: str,
    level: str,
    target: str,
) -> list[str]:
    """Emit a Rust-side log record and capture stderr output."""
    _handler, logger_name = handler_ctx
    rust._emit_rust_log(level, message, target)
    logger = get_logger(logger_name)
    logger.flush_handlers()

    err_lines = capfd.readouterr().err.strip().splitlines()
    prefix = f"{logger_name} [{level.upper()}] "
    return [ln for ln in err_lines if ln.startswith(prefix)]


@then("the captured stderr output matches snapshot")
def then_stderr_snapshot(output: Sequence[str], snapshot: SnapshotAssertion) -> None:
    """Assert captured stderr output equals the stored snapshot."""
    assert output == snapshot, "stderr output must match snapshot"
