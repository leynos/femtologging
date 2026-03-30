"""BDD steps for Rust tracing compatibility."""

from __future__ import annotations

import contextlib
import os
import subprocess  # noqa: S404 - FIXME: required to test fresh-process global subscriber semantics.
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
    setup_rust_tracing,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy.assertion import SnapshotAssertion

    from femtologging.adapter import FemtoRecord

    Iterator = cabc.Iterator
    Sequence = cabc.Sequence

FEATURES = Path(__file__).resolve().parents[1] / "features"

REQUIRED_RUST_ATTRS = (
    "setup_rust_tracing",
    "_emit_rust_tracing_event",
    "_emit_rust_tracing_structured_event",
    "_emit_rust_tracing_span_event",
    "_install_test_global_tracing_subscriber",
)

pytestmark = [pytest.mark.tracing_compat]

if not all(hasattr(rust, attr) for attr in REQUIRED_RUST_ATTRS):
    pytest.skip(
        "tracing-compat feature not built; tracing bridge helpers unavailable",
        allow_module_level=True,
    )

scenarios(str(FEATURES / "rust_tracing_compat.feature"))


@given(
    parsers.parse('a stream handler attached to logger "{name}"'),
    target_fixture="stream_handler_ctx",
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


class _RecordCaptureHandler:
    """Collect record payloads received through ``handle_record``."""

    def __init__(self) -> None:
        self.records: list[FemtoRecord] = []

    def handle(self, _message: str, _logger_name: str, _level: str) -> None:
        """Satisfy handler registration; structured dispatch uses handle_record."""

    def handle_record(self, record: FemtoRecord) -> None:
        self.records.append(record)

    def close(self) -> None:
        """Provide a stdlib-like close hook for teardown symmetry."""


@given(
    parsers.parse('a record-collecting handler attached to logger "{name}"'),
    target_fixture="record_handler_ctx",
)
def given_record_handler(name: str) -> Iterator[tuple[_RecordCaptureHandler, str]]:
    """Attach a Python ``handle_record`` collector to the named logger."""
    handler = _RecordCaptureHandler()
    logger = get_logger(name)
    logger.add_handler(handler)
    try:
        yield handler, name
    finally:
        logger.clear_handlers()
        handler.close()


@when("I set up rust tracing bridge")
def when_setup_tracing_bridge() -> None:
    """Install the Rust tracing subscriber bridge."""
    setup_rust_tracing()


@when(parsers.parse('I set logger "{name}" level to "{level}"'))
def when_set_logger_level(name: str, level: str) -> None:
    """Change the named logger's minimum level for the scenario."""
    logger = get_logger(name)
    logger.set_level(level)


def _flush_and_sync_stderr(logger: FemtoLogger, handler: FemtoStreamHandler) -> None:
    """Flush the logger and handler queues and sync stderr to the OS."""
    logger.flush_handlers()
    handler.flush()
    with contextlib.suppress(OSError):
        sys.stderr.flush()
    with contextlib.suppress(OSError):
        os.fsync(sys.stderr.fileno())


@when(
    parsers.parse('I emit a Rust tracing event "{message}" at "{level}"'),
    target_fixture="stderr_output",
)
def when_emit_rust_tracing_event(
    stream_handler_ctx: tuple[FemtoStreamHandler, str],
    capfd: pytest.CaptureFixture[str],
    *,
    message: str,
    level: str,
) -> list[str]:
    """Emit a Rust tracing event and capture stderr output."""
    capfd.readouterr()
    handler, logger_name = stream_handler_ctx
    rust._emit_rust_tracing_event(level, message)
    logger = get_logger(logger_name)
    _flush_and_sync_stderr(logger, handler)

    prefix = f"{logger_name} [{level.upper()}] "
    deadline = time.monotonic() + 2.0
    captured: list[str] = []
    while time.monotonic() < deadline:
        err = capfd.readouterr().err
        if err:
            captured.extend(err.strip().splitlines())
        matching = [line for line in captured if line.startswith(prefix)]
        if matching:
            return matching
        time.sleep(0.01)
    return [line for line in captured if line.startswith(prefix)]


def _flush_record_handler(logger_name: str) -> None:
    logger = get_logger(logger_name)
    logger.flush_handlers()


def _snapshot_record(record: FemtoRecord) -> dict[str, object]:
    metadata = typ.cast("dict[str, object]", record.get("metadata", {}))
    key_values = typ.cast("dict[str, object]", metadata.get("key_values", {}))
    return {
        "logger": record.get("logger"),
        "level": record.get("level"),
        "message": record.get("message"),
        "key_values": key_values,
    }


@when(
    "I emit a structured Rust tracing event",
    target_fixture="captured_tracing_records",
)
def when_emit_structured_rust_tracing_event(
    record_handler_ctx: tuple[_RecordCaptureHandler, str],
) -> list[dict[str, object]]:
    """Emit a structured tracing event and capture the Python record payload."""
    handler, logger_name = record_handler_ctx
    rust._emit_rust_tracing_structured_event()
    _flush_record_handler(logger_name)
    return [_snapshot_record(record) for record in handler.records]


@when(
    "I emit a nested Rust tracing span event",
    target_fixture="captured_tracing_records",
)
def when_emit_nested_rust_tracing_span_event(
    record_handler_ctx: tuple[_RecordCaptureHandler, str],
) -> list[dict[str, object]]:
    """Emit a nested-span tracing event and capture the Python record payload."""
    handler, logger_name = record_handler_ctx
    rust._emit_rust_tracing_span_event()
    _flush_record_handler(logger_name)
    return [_snapshot_record(record) for record in handler.records]


@when(
    "I attempt to set up rust tracing bridge in a fresh process and it fails",
    target_fixture="tracing_bridge_error",
)
def when_setup_tracing_bridge_fails_in_subprocess() -> list[str]:
    """Run tracing setup in a fresh process and assert it fails as expected."""
    script = """
import sys

import femtologging
from femtologging import rust

rust._install_test_global_tracing_subscriber()

try:
    femtologging.setup_rust_tracing()
except RuntimeError as exc:
    print(str(exc))
    raise SystemExit(0)

print("setup_rust_tracing unexpectedly succeeded")
raise SystemExit(1)
""".strip()
    result = subprocess.run(  # noqa: S603 - FIXME: required to validate subprocess failure semantics.
        [sys.executable, "-c", script],
        check=False,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, (
        "expected setup_rust_tracing to fail in the subprocess; "
        f"rc={result.returncode}, stdout={result.stdout!r}, stderr={result.stderr!r}"
    )
    return result.stdout.strip().splitlines()


@then("the captured tracing stderr output matches snapshot")
def then_tracing_stderr_snapshot(
    stderr_output: Sequence[str],
    snapshot: SnapshotAssertion,
) -> None:
    """Assert captured tracing stderr output equals the stored snapshot."""
    assert stderr_output == snapshot, "stderr output must match snapshot"


@then("the captured tracing records match snapshot")
def then_tracing_records_snapshot(
    captured_tracing_records: Sequence[dict[str, object]],
    snapshot: SnapshotAssertion,
) -> None:
    """Assert captured tracing records equal the stored snapshot."""
    assert captured_tracing_records == snapshot, "record payloads must match snapshot"


@then("the rust tracing bridge error matches snapshot")
def then_tracing_bridge_error_snapshot(
    tracing_bridge_error: Sequence[str],
    snapshot: SnapshotAssertion,
) -> None:
    """Assert bridge failure output equals the stored snapshot."""
    assert tracing_bridge_error == snapshot, "error output must match snapshot"
