"""BDD steps for Send/Sync guarantees."""

from __future__ import annotations

import threading
from pathlib import Path
import typing as typ

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import FemtoStreamHandler, StreamHandlerBuilder

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from syrupy.assertion import SnapshotAssertion
    Iterator = cabc.Iterator
    Sequence = cabc.Sequence

INFO_PREFIX = "test [INFO] "

FEATURES = Path(__file__).resolve().parents[1] / "features"

pytestmark = [pytest.mark.send_sync, pytest.mark.concurrency]

scenarios(str(FEATURES / "send_sync.feature"))


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
