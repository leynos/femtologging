"""Behavioural tests for Send/Sync guarantees."""

from __future__ import annotations

import threading

from pytest_bdd import given, scenarios, then, when

from femtologging import StreamHandlerBuilder


@given("a stream handler built for stderr", target_fixture="handler")
def given_handler() -> object:
    return StreamHandlerBuilder.stderr().build()


@given("the handler is closed")
def given_closed(handler) -> None:
    handler.close()


@when("I log a message", target_fixture="output")
def when_log_one(handler, capfd) -> str:
    handler.handle("test", "INFO", "drop me")
    handler.flush()
    return capfd.readouterr().err


@when("I log messages from 3 threads", target_fixture="output")
def when_log_threads(handler, capfd) -> list[str]:
    def worker(i: int) -> None:
        handler.handle("test", "INFO", f"message {i}")

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(3)]
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
def then_output_snapshot(output: str, snapshot) -> None:
    assert output == snapshot


scenarios("features/send_sync.feature")
