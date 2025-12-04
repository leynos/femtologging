"""Behavioural tests for Send/Sync guarantees."""

from __future__ import annotations

import threading
from typing import TYPE_CHECKING

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import FemtoStreamHandler, StreamHandlerBuilder

if TYPE_CHECKING:
    from collections.abc import Iterator, Sequence

    from syrupy.assertion import SnapshotAssertion

INFO_PREFIX = "test [INFO] "


pytestmark = [pytest.mark.send_sync, pytest.mark.concurrency]


@given("a stream handler built for stderr", target_fixture="handler")
def given_handler() -> Iterator[FemtoStreamHandler]:
    handler = StreamHandlerBuilder.stderr().build()
    try:
        yield handler
    finally:
        handler.close()


@given("the handler is closed")
def given_closed(handler: FemtoStreamHandler) -> None:
    handler.close()


@when("I log a message", target_fixture="output")
def when_log_one(
    handler: FemtoStreamHandler, capfd: pytest.CaptureFixture[str]
) -> list[str]:
    handler.handle("test", "INFO", "drop me")
    ok = handler.flush()
    out = capfd.readouterr().err.strip().splitlines()
    lines = [ln for ln in out if ln.startswith(INFO_PREFIX)]
    assert ok, "handler.flush() timed out"
    return lines


@when("I log a message after closing", target_fixture="output")
def when_log_after_close(
    handler: FemtoStreamHandler, capfd: pytest.CaptureFixture[str]
) -> list[str]:
    with pytest.raises(RuntimeError, match="Handler error: handler is closed"):
        handler.handle("test", "INFO", "drop me")
    ok = handler.flush()
    out = capfd.readouterr().err.strip().splitlines()
    lines = [ln for ln in out if ln.startswith(INFO_PREFIX)]
    assert not ok, "handler.flush() unexpectedly succeeded"
    return lines


@when(
    parsers.parse("I log messages from {count:d} threads"),
    target_fixture="output",
)
def when_log_threads(
    handler: FemtoStreamHandler,
    capfd: pytest.CaptureFixture[str],
    count: int,
) -> list[str]:
    def worker(i: int) -> None:
        handler.handle("test", "INFO", f"message {i}")

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(count)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    ok = handler.flush()
    out = capfd.readouterr().err.strip().splitlines()
    assert ok, "handler.flush() timed out"
    lines = [ln for ln in out if ln.startswith(INFO_PREFIX)]

    # For counts >= 10, ensure "message 10" sorts after "message 9".
    # Fall back to lexicographic order if numeric suffix is missing.
    def _suffix_num(s: str) -> int | None:
        try:
            return int(s.rsplit(" ", 1)[-1])
        except (ValueError, IndexError):
            return None

    lines.sort(key=lambda s: (_suffix_num(s) is None, _suffix_num(s) or 0, s))
    return lines


@then("the captured output matches snapshot")
def then_output_snapshot(output: Sequence[str], snapshot: SnapshotAssertion) -> None:
    assert output == snapshot, "normalised output does not match the snapshot"


@pytest.mark.parametrize("thread_count", [1, 10, 100])
def test_threaded_logging(thread_count: int, capfd: pytest.CaptureFixture[str]) -> None:
    handler: FemtoStreamHandler = StreamHandlerBuilder.stderr().build()
    try:
        threads = [
            threading.Thread(target=lambda: handler.handle("test", "INFO", "msg"))
            for _ in range(thread_count)
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        ok = handler.flush()
        out_lines = capfd.readouterr().err.strip().splitlines()
        assert ok, "handler.flush() timed out"
        info_lines = [ln for ln in out_lines if ln.startswith(INFO_PREFIX)]
        assert len(info_lines) == thread_count, (
            f"expected {thread_count} '{INFO_PREFIX}' lines, "
            f"got {len(info_lines)}; all lines: {out_lines}"
        )
    finally:
        handler.close()


scenarios("features/send_sync.feature")
