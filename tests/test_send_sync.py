"""Behavioural tests for Send/Sync guarantees."""

from __future__ import annotations

from collections.abc import Sequence
import threading

import pytest
from pytest_bdd import given, scenarios, then, when, parsers

from femtologging import FemtoStreamHandler, StreamHandlerBuilder


@given("a stream handler built for stderr", target_fixture="handler")
def given_handler() -> FemtoStreamHandler:
    return StreamHandlerBuilder.stderr().build()


@given("the handler is closed")
def given_closed(handler: FemtoStreamHandler) -> None:
    handler.close()


@when("I log a message", target_fixture="output")
def when_log_one(
    handler: FemtoStreamHandler, capfd: pytest.CaptureFixture[str]
) -> list[str]:
    handler.handle("test", "INFO", "drop me")
    handler.flush()
    return capfd.readouterr().err.strip().splitlines()


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
    handler.flush()
    out = capfd.readouterr().err.strip()
    lines = out.splitlines()
    lines.sort()
    return lines


@then("the captured output matches snapshot")
def then_output_snapshot(output: Sequence[str], snapshot) -> None:
    assert output == snapshot


@pytest.mark.parametrize("thread_count", [1, 10, 100])
def test_threaded_logging(thread_count: int, capfd: pytest.CaptureFixture[str]) -> None:
    handler: FemtoStreamHandler = StreamHandlerBuilder.stderr().build()
    threads = [
        threading.Thread(target=lambda: handler.handle("test", "INFO", "msg"))
        for _ in range(thread_count)
    ]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    handler.flush()
    out = capfd.readouterr().err.strip().splitlines()
    assert len(out) == thread_count, f"expected {thread_count} lines, got {len(out)}"


scenarios("features/send_sync.feature")
