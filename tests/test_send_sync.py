"""Behavioural tests for Send/Sync guarantees."""

from __future__ import annotations

from collections.abc import Sequence
from typing import Iterator
import threading

import pytest
from pytest_bdd import given, scenarios, then, when, parsers

from femtologging import FemtoStreamHandler, StreamHandlerBuilder


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
    out = capfd.readouterr().err.strip().splitlines()
    lines = [ln for ln in out if ln.startswith("test [INFO] ")]
    lines.sort()
    return lines


@then("the captured output matches snapshot")
def then_output_snapshot(output: Sequence[str], snapshot) -> None:
    assert output == snapshot


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
        handler.flush()
        out_lines = capfd.readouterr().err.strip().splitlines()
        info_lines = [ln for ln in out_lines if ln.startswith("test [INFO] ")]
        assert len(info_lines) == thread_count, (
            f"expected {thread_count} 'test [INFO] ' lines, "
            f"got {len(info_lines)}; all lines: {out_lines}"
        )
    finally:
        handler.close()


scenarios("features/send_sync.feature")
