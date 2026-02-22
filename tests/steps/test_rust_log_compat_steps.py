"""BDD steps for Rust log crate compatibility."""

from __future__ import annotations

import contextlib
import dataclasses as dc
import os
import subprocess  # noqa: S404 - FIXME: required to test fresh-process global logger semantics.
import sys
import time
import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    FemtoLogger,
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

REQUIRED_RUST_ATTRS = (
    "setup_rust_logging",
    "_emit_rust_log",
    "_install_test_global_rust_logger",
)

pytestmark = [pytest.mark.log_compat]

if not all(hasattr(rust, attr) for attr in REQUIRED_RUST_ATTRS):
    pytest.skip(
        "log-compat feature not built; rust bridge helpers unavailable",
        allow_module_level=True,
    )

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


@dc.dataclass(slots=True, frozen=True)
class RustLogParams:
    """Parameters for emitting a Rust log record."""

    message: str
    level: str
    target: str


def _flush_and_sync_stderr(
    logger: FemtoLogger,
    handler: FemtoStreamHandler,
) -> None:
    """Flush the logger and handler queues and sync stderr to the OS.

    Best-effort: drains enqueued records so that the file descriptor
    contains all output before the caller captures it.  Return values
    are intentionally ignored because the Rust log-bridge path may
    dispatch records through a different internal channel from the one
    the Python-side logger exposes.
    """
    logger.flush_handlers()
    handler.flush()
    with contextlib.suppress(OSError):
        os.fsync(sys.stderr.fileno())


def when_emit_rust_log(
    handler_ctx: tuple[FemtoStreamHandler, str],
    capfd: pytest.CaptureFixture[str],
    log_params: RustLogParams,
) -> list[str]:
    """Emit a Rust-side log record and capture stderr output."""
    capfd.readouterr()
    handler, logger_name = handler_ctx
    rust._emit_rust_log(log_params.level, log_params.message, log_params.target)
    logger = get_logger(logger_name)
    _flush_and_sync_stderr(logger, handler)

    prefix = f"{logger_name} [{log_params.level.upper()}] "
    deadline = time.monotonic() + 2.0
    captured: list[str] = []
    while time.monotonic() < deadline:
        err = capfd.readouterr().err
        if err:
            captured.extend(err.strip().splitlines())
        matching = [ln for ln in captured if ln.startswith(prefix)]
        if matching:
            return matching
        time.sleep(0.01)
    return [ln for ln in captured if ln.startswith(prefix)]


@pytest.fixture
def rust_log_test_ctx(
    handler_ctx: tuple[FemtoStreamHandler, str],
    capfd: pytest.CaptureFixture[str],
) -> tuple[tuple[FemtoStreamHandler, str], pytest.CaptureFixture[str]]:
    """Return combined test context for Rust log compatibility tests."""
    return handler_ctx, capfd


@when(
    parsers.parse('I emit a Rust log "{message}" at "{level}" with target "{target}"'),
    target_fixture="output",
)
def when_emit_rust_log_step(
    rust_log_test_ctx: tuple[
        tuple[FemtoStreamHandler, str], pytest.CaptureFixture[str]
    ],
    *,
    message: str,
    level: str,
    target: str,
) -> list[str]:
    """Emit a Rust-side log record and capture stderr output."""
    handler_ctx, capfd = rust_log_test_ctx
    return when_emit_rust_log(
        handler_ctx,
        capfd,
        RustLogParams(message=message, level=level, target=target),
    )


@when(
    "I attempt to set up rust logging bridge in a fresh process and it fails",
    target_fixture="rust_bridge_error",
)
def when_setup_bridge_fails_in_subprocess() -> list[str]:
    """Run setup in a fresh process and assert it fails as expected."""
    script = """
import sys

import femtologging
from femtologging import rust

rust._install_test_global_rust_logger()

try:
    femtologging.setup_rust_logging()
except RuntimeError as exc:
    print(str(exc))
    raise SystemExit(0)

print("setup_rust_logging unexpectedly succeeded")
raise SystemExit(1)
""".strip()
    result = subprocess.run(  # noqa: S603 - FIXME: required to validate subprocess failure semantics.
        [sys.executable, "-c", script],
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, (
        "expected setup_rust_logging to fail in the subprocess; "
        f"rc={result.returncode}, stdout={result.stdout!r}, stderr={result.stderr!r}"
    )
    return result.stdout.strip().splitlines()


@then("the rust logging bridge error matches snapshot")
def then_bridge_error_snapshot(
    rust_bridge_error: Sequence[str],
    snapshot: SnapshotAssertion,
) -> None:
    """Assert bridge failure output equals the stored snapshot."""
    assert rust_bridge_error == snapshot, "error output must match snapshot"


@then("the captured stderr output matches snapshot")
def then_stderr_snapshot(output: Sequence[str], snapshot: SnapshotAssertion) -> None:
    """Assert captured stderr output equals the stored snapshot."""
    assert output == snapshot, "stderr output must match snapshot"
