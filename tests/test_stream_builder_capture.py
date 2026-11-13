"""Capture tests for the stream handler builder's callable formatter.

These live apart from the other builder tests because they need stdio capture
and a real worker: each case has to prove the worker finished before stderr is
read, which is more machinery than the configuration-only builder tests use.
"""

from __future__ import annotations

import time
import typing as typ

import pytest

from femtologging import StreamHandlerBuilder

# Configured close timeout. It is far longer than the time a callable formatter
# needs, so a close that returns within the ceiling below must have observed
# worker completion rather than timed out.
_CLOSE_TIMEOUT_MS = 5_000
_CLOSE_RETURN_CEILING_SECONDS = 1.0


@pytest.mark.parametrize("drain_method", ["flush", "close"], ids=["flush", "close"])
def test_stream_builder_applies_callable_formatter(
    capfd: pytest.CaptureFixture[str],
    drain_method: typ.Literal["flush", "close"],
) -> None:
    """Stream workers can finish Python formatting during flush and close."""
    handler = (
        StreamHandlerBuilder
        .stderr()
        .with_flush_after_ms(_CLOSE_TIMEOUT_MS)
        .with_formatter(lambda record: f"callable:{record['message']}")
        .build()
    )
    try:
        handler.handle("logger", "INFO", "hello")
        started = time.monotonic()
        if drain_method == "flush":
            assert handler.flush(), "stream handler did not flush"
        else:
            handler.close()
            # close() returns either when the worker completed or when its
            # timeout expired. Returning well inside the configured timeout
            # therefore proves completion, so the capture below cannot be
            # racing a write the worker has yet to make.
            elapsed = time.monotonic() - started
            assert elapsed < _CLOSE_RETURN_CEILING_SECONDS, (
                f"close() returned after {elapsed:.3f}s, at or near its "
                f"{_CLOSE_TIMEOUT_MS}ms timeout; the worker may still be writing"
            )
    finally:
        handler.close()

    # Completion means every earlier record reached the stream, so one read is
    # exact: no polling and no destructive drain while the worker can write.
    captured = capfd.readouterr().err
    assert "callable:hello\n" in captured, f"stderr={captured!r}"
