"""BDD steps for Send/Sync guarantees."""

from __future__ import annotations

import threading
import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import FemtoStreamHandler, StreamHandlerBuilder

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy.assertion import SnapshotAssertion

INFO_PREFIX = "test [INFO] "

FEATURES = Path(__file__).resolve().parents[1] / "features"

pytestmark = [pytest.mark.send_sync, pytest.mark.concurrency]

scenarios(str(FEATURES / "send_sync.feature"))


@given("a stream handler built for stderr", target_fixture="handler")
def given_handler() -> cabc.Iterator[FemtoStreamHandler]:
    handler = StreamHandlerBuilder.stderr().build()
    try:
        yield handler
    finally:
        handler.close()


@given("the handler is closed")
def given_closed(handler: FemtoStreamHandler) -> None:
    handler.close()


def _drain_info_lines(capfd: pytest.CaptureFixture[str]) -> list[str]:
    """Return the captured stderr lines emitted at INFO by the test logger."""
    captured = capfd.readouterr().err.strip().splitlines()
    return [line for line in captured if line.startswith(INFO_PREFIX)]


@when("I log a message", target_fixture="output")
def when_log_one(
    handler: FemtoStreamHandler, capfd: pytest.CaptureFixture[str]
) -> list[str]:
    handler.handle("test", "INFO", "drop me")
    is_flushed = handler.flush()
    lines = _drain_info_lines(capfd)
    assert is_flushed, "an open handler must drain its queue before the flush deadline"
    return lines


@when("I log a message after closing", target_fixture="output")
def when_log_after_close(
    handler: FemtoStreamHandler, capfd: pytest.CaptureFixture[str]
) -> list[str]:
    with pytest.raises(RuntimeError, match="Handler error: handler is closed"):
        handler.handle("test", "INFO", "drop me")
    is_flushed = handler.flush()
    lines = _drain_info_lines(capfd)
    assert not is_flushed, "flushing a closed handler must report failure, not success"
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
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    is_flushed = handler.flush()
    lines = _drain_info_lines(capfd)
    assert is_flushed, (
        "the handler must drain every thread's record before the flush deadline"
    )

    def _suffix_num(line: str) -> int | None:
        try:
            return int(line.rsplit(" ", 1)[-1])
        except (ValueError, IndexError):
            return None

    # Concurrent writers interleave non-deterministically, so order by the
    # message ordinal to give the snapshot a stable sequence.
    lines.sort(key=lambda s: (_suffix_num(s) is None, _suffix_num(s) or 0, s))
    return lines


@then("the captured output matches snapshot")
def then_output_snapshot(
    output: cabc.Sequence[str], snapshot: SnapshotAssertion
) -> None:
    assert output == snapshot, (
        "concurrently emitted records must match the recorded snapshot "
        f"once normalized; got {output!r}"
    )
