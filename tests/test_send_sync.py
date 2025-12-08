"""Behavioural tests for Send/Sync guarantees."""

from __future__ import annotations

import threading

import pytest

from femtologging import FemtoStreamHandler, StreamHandlerBuilder

INFO_PREFIX = "test [INFO] "


pytestmark = [pytest.mark.send_sync, pytest.mark.concurrency]


@pytest.mark.parametrize("thread_count", [1, 10, 100])
def test_threaded_logging(thread_count: int, capfd: pytest.CaptureFixture[str]) -> None:
    """Stream handler should safely handle concurrent logging."""
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
