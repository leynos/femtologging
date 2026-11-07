"""Regression tests for stream-handler shutdown ordering.

A stream worker runs Python callable formatters on its own thread. The
Python-facing ``close()`` and ``flush()`` bindings must release the GIL while
they wait for that worker, otherwise the formatter cannot run and the binding
times out. These tests gate the formatter on an event so they observe the
ordering directly: the native call may not return until the formatter has
completed and its record has reached the stream.
"""

from __future__ import annotations

import threading
import time
import typing as typ

from femtologging import StreamHandlerBuilder

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    import pytest

# Long enough to distinguish a correctly released wait (tens of milliseconds)
# from a GIL-held wait that runs to the handler's configured timeout.
_GATE_TIMEOUT_SECONDS = 10.0
_RELEASE_DELAY_SECONDS = 0.05
_DRAIN_TIMEOUT_MS = 5_000
_MAX_DRAIN_SECONDS = 3.0


class _FormatterNotReleasedError(Exception):
    """The test formatter was not released before its timeout."""

    def __init__(self) -> None:
        super().__init__("test formatter was not released")


class _StreamHandler(typ.Protocol):
    """Python methods shared by the worker-backed stream handler."""

    def handle(self, logger: str, level: str, message: str) -> None: ...

    def flush(self) -> bool: ...

    def close(self) -> None: ...


class _FormatterGate:
    """Coordinate a callable formatter that blocks until the test releases it."""

    def __init__(self) -> None:
        self.entered = threading.Event()
        self.release = threading.Event()
        self.completed = threading.Event()

    def format_record(self, record: cabc.Mapping[str, object]) -> str:
        """Block until released, then return a distinctive formatted line."""
        self.entered.set()
        if not self.release.wait(timeout=_GATE_TIMEOUT_SECONDS):
            raise _FormatterNotReleasedError
        self.completed.set()
        return f"gated:{record['message']}"

    def release_after_delay(self) -> threading.Thread:
        """Start a helper thread that releases the formatter after a short delay."""

        def release_gate() -> None:
            time.sleep(_RELEASE_DELAY_SECONDS)
            self.release.set()

        thread = threading.Thread(
            target=release_gate,
            name="formatter-release",
            daemon=True,
        )
        thread.start()
        return thread


def _build_gated_handler(gate: _FormatterGate) -> _StreamHandler:
    """Build a stderr handler whose formatter blocks until the gate releases."""
    handler = (
        StreamHandlerBuilder
        .stderr()
        .with_flush_after_ms(_DRAIN_TIMEOUT_MS)
        .with_formatter(gate.format_record)
        .build()
    )
    return typ.cast("_StreamHandler", handler)


def test_stream_close_waits_for_callable_formatter(
    capfd: pytest.CaptureFixture[str],
) -> None:
    """close() releases the GIL and orders the worker's write."""
    gate = _FormatterGate()
    handler = _build_gated_handler(gate)
    releaser: threading.Thread | None = None
    completed = False
    elapsed = 0.0
    captured = ""
    try:
        handler.handle("logger", "INFO", "hello")
        assert gate.entered.wait(timeout=_GATE_TIMEOUT_SECONDS), (
            "worker never entered the callable formatter"
        )
        releaser = gate.release_after_delay()
        started = time.monotonic()
        handler.close()
        elapsed = time.monotonic() - started
        completed = gate.completed.is_set()
        captured = capfd.readouterr().err
    finally:
        gate.release.set()
        handler.close()
        if releaser is not None:
            releaser.join(timeout=_GATE_TIMEOUT_SECONDS)

    assert completed, "close() returned before the formatter completed"
    assert elapsed < _MAX_DRAIN_SECONDS, (
        f"close() waited {elapsed:.3f}s; the GIL was not released"
    )
    assert "gated:hello\n" in captured, (
        f"close() should order the worker write; stderr={captured!r}"
    )


def test_stream_flush_waits_for_callable_formatter(
    capfd: pytest.CaptureFixture[str],
) -> None:
    """flush() releases the GIL and reports the completed write."""
    gate = _FormatterGate()
    handler = _build_gated_handler(gate)
    releaser: threading.Thread | None = None
    flushed = False
    completed = False
    elapsed = 0.0
    captured = ""
    try:
        handler.handle("logger", "INFO", "hello")
        assert gate.entered.wait(timeout=_GATE_TIMEOUT_SECONDS), (
            "worker never entered the callable formatter"
        )
        releaser = gate.release_after_delay()
        started = time.monotonic()
        flushed = handler.flush()
        elapsed = time.monotonic() - started
        completed = gate.completed.is_set()
        captured = capfd.readouterr().err
    finally:
        gate.release.set()
        handler.close()
        if releaser is not None:
            releaser.join(timeout=_GATE_TIMEOUT_SECONDS)

    assert flushed is True, "flush() did not acknowledge the worker"
    assert completed, "flush() returned before the formatter completed"
    assert elapsed < _MAX_DRAIN_SECONDS, (
        f"flush() waited {elapsed:.3f}s; the GIL was not released"
    )
    assert "gated:hello\n" in captured, (
        f"flush() should order the worker write; stderr={captured!r}"
    )
