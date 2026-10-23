"""BDD steps for Rust log crate compatibility."""

from __future__ import annotations

import contextlib
import dataclasses as dc
import os
import subprocess  # ruff: ignore[suspicious-subprocess-import] a fresh interpreter is the only way to exercise install-once global logger semantics
import sys
import time
import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    ConfigBuilder,
    FemtoLogger,
    FemtoStreamHandler,
    LoggerConfigBuilder,
    LoggerMutationBuilder,
    RuntimeConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
    reset_manager,
    rust,
    setup_rust_logging,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy.assertion import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

REQUIRED_RUST_ATTRS = (
    "setup_rust_logging",
    "_emit_rust_log",
    "_rust_log_enabled",
    "_rust_log_max_level",
    "_install_test_global_rust_logger",
)

pytestmark = [pytest.mark.log_compat]

if not all(hasattr(rust, attr) for attr in REQUIRED_RUST_ATTRS):
    missing_attrs = sorted(
        attr for attr in REQUIRED_RUST_ATTRS if not hasattr(rust, attr)
    )
    message = (
        f"Rust log compatibility bridge missing required attributes: {missing_attrs}"
    )
    skip = typ.cast("typ.Any", pytest.skip)
    skip(message, allow_module_level=True)

scenarios(str(FEATURES / "rust_log_compat.feature"))


@given(
    parsers.parse('a stream handler attached to logger "{name}"'),
    target_fixture="handler_ctx",
)
def given_stream_handler(name: str) -> cabc.Iterator[tuple[FemtoStreamHandler, str]]:
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


@when("I reset the logger manager")
def when_reset_logger_manager() -> None:
    """Clear registered loggers and restore the bridge's default maximum."""
    reset_manager()


@when(parsers.parse('I configure logger "{name}" to "{level}" via build_and_init'))
def when_configure_logger_level(name: str, level: str) -> None:
    """Set a logger threshold through the configuration builder."""
    (
        ConfigBuilder()
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        .with_logger(name, LoggerConfigBuilder().with_level(level))
        .build_and_init()
    )


@when(parsers.parse('I set logger "{name}" level to "{level}" via runtime mutation'))
def when_mutate_logger_level(name: str, level: str) -> None:
    """Set a logger threshold through the runtime mutation API."""
    RuntimeConfigBuilder().with_logger(
        name,
        LoggerMutationBuilder().with_level(level),
    ).apply()


@then(
    parsers.parse('the Rust log levels "{levels}" are disabled for target "{target}"')
)
def then_log_levels_are_disabled(levels: str, target: str) -> None:
    """Assert the Rust log facade rejects every listed target level."""
    enabled_levels = [
        level
        for level in levels.split(",")
        if rust._rust_log_enabled(level.strip(), target)
    ]
    assert not enabled_levels, (
        f"expected the target {target!r} to reject {levels}, "
        f"but enabled {enabled_levels!r}"
    )


@then(parsers.parse('the Rust log levels "{levels}" are enabled for target "{target}"'))
def then_log_levels_are_enabled(levels: str, target: str) -> None:
    """Assert the Rust log facade accepts every listed target level."""
    disabled_levels = [
        level
        for level in levels.split(",")
        if not rust._rust_log_enabled(level.strip(), target)
    ]
    assert not disabled_levels, (
        f"expected the target {target!r} to accept {levels}, "
        f"but disabled {disabled_levels!r}"
    )


@then(parsers.parse('the Rust log facade maximum is "{level}"'))
def then_rust_log_max_level_is(level: str) -> None:
    """Assert the process-wide Rust log macro prefilter threshold."""
    actual = rust._rust_log_max_level()
    assert actual == level, f"expected facade maximum {level!r}, got {actual!r}"


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
        sys.stderr.flush()
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
    result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed interpreter path and inline script
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
    rust_bridge_error: cabc.Sequence[str],
    snapshot: SnapshotAssertion,
) -> None:
    """Assert bridge failure output equals the stored snapshot."""
    assert rust_bridge_error == snapshot, (
        "the double-install failure message must match the recorded snapshot; "
        f"got {rust_bridge_error!r}"
    )


@then("the captured stderr output matches snapshot")
def then_stderr_snapshot(
    output: cabc.Sequence[str], snapshot: SnapshotAssertion
) -> None:
    """Assert captured stderr output equals the stored snapshot."""
    assert output == snapshot, (
        "records bridged from the Rust log crate must match the recorded "
        f"snapshot; got {output!r}"
    )
