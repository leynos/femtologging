"""Regression tests for stream-handler shutdown ordering.

A stream worker runs Python callable formatters on its own thread. The
Python-facing ``close()`` and ``flush()`` bindings must release the GIL while
they wait for that worker, otherwise the formatter cannot run and the binding
times out. This test gates the formatter on an event so it observes the
ordering directly: the native call may not return until the formatter has
completed and its record has reached the stream.
"""

from __future__ import annotations

import dataclasses
import threading
import time
import typing as typ

import pytest

from femtologging import StreamHandlerBuilder, rust
from tests.helpers import poll_captured_text

if typ.TYPE_CHECKING:
    import collections.abc as cabc

# Long enough to distinguish a correctly released wait (tens of milliseconds)
# from a GIL-held wait that runs to the handler's configured timeout.
_GATE_TIMEOUT_SECONDS = 10.0
_WAITER_TIMEOUT_SECONDS = 5.0
_WAITER_POLL_SECONDS = 0.005
_DRAIN_TIMEOUT_MS = 5_000
_MAX_DRAIN_SECONDS = 3.0

# Short configured close timeout for the timeout-path test, with a floor below
# it and a ceiling well above it so scheduling jitter cannot decide the result.
_CLOSE_TIMEOUT_MS = 150
_CLOSE_WAIT_FLOOR_SECONDS = _CLOSE_TIMEOUT_MS / 1000 / 2
_CLOSE_WAIT_CEILING_SECONDS = 5.0

# Reports how many native stream-handler calls are blocked in a wait. It is
# present only when the extension was built with the Rust `test-util` feature,
# which `make develop` enables for the test suite.
_WAITER_COUNT: cabc.Callable[[], int] | None = getattr(
    rust, "_stream_waiter_count", None
)

pytestmark = pytest.mark.skipif(
    _WAITER_COUNT is None,
    reason="requires Rust extension built with the 'test-util' feature",
)

type _DrainMethod = typ.Literal["flush", "close"]


class _FormatterNotReleasedError(Exception):
    """The test formatter was not released before its timeout."""

    def __init__(self) -> None:
        """Record that the formatter gate timed out before release."""
        super().__init__("test formatter was not released")


class _WaiterNotObservedError(Exception):
    """No native flush or close call was observed waiting before the deadline."""

    def __init__(self) -> None:
        """Record that the waiter handshake timed out."""
        super().__init__("no native stream-handler call was observed waiting")


class _StreamHandler(typ.Protocol):
    """Python methods shared by the worker-backed stream handler."""

    def handle(self, logger: str, level: str, message: str) -> None:
        """Queue one record for the worker."""

    def flush(self) -> bool:
        """Drain queued records and report whether the worker acknowledged."""

    def close(self) -> None:
        """Stop accepting records and wait for the worker to finish."""


class _FormatterGate:
    """Coordinate a callable formatter that blocks until the test releases it."""

    def __init__(self) -> None:
        """Create the events that gate and observe the formatter."""
        self.entered = threading.Event()
        self.release = threading.Event()
        self.completed = threading.Event()
        self.waiter_observed = threading.Event()
        self.waiter_error: Exception | None = None

    def format_record(self, record: cabc.Mapping[str, object]) -> str:
        """Block until released, then return a distinctive formatted line."""
        self.entered.set()
        if not self.release.wait(timeout=_GATE_TIMEOUT_SECONDS):
            raise _FormatterNotReleasedError
        self.completed.set()
        return f"gated:{record['message']}"

    def release_when_waiting(self) -> threading.Thread:
        """Start a helper thread that releases the formatter once a drain waits.

        The helper polls the native waiter counter, so the formatter is only
        released after `flush()` or `close()` has entered its wait. A binding
        that held the GIL could not let this thread run at all, which is the
        regression the test exists to catch.

        Returns
        -------
        threading.Thread
            The started helper thread.
        """

        def release_gate() -> None:
            """Poll for a waiting native call, then release the formatter."""
            waiter_count = _WAITER_COUNT
            if waiter_count is None:
                self.waiter_error = _WaiterNotObservedError()
                return
            deadline = time.monotonic() + _WAITER_TIMEOUT_SECONDS
            while time.monotonic() < deadline:
                if waiter_count() > 0:
                    self.waiter_observed.set()
                    self.release.set()
                    return
                time.sleep(_WAITER_POLL_SECONDS)
            self.waiter_error = _WaiterNotObservedError()

        thread = threading.Thread(
            target=release_gate,
            name="formatter-release",
            daemon=True,
        )
        thread.start()
        return thread


def _build_gated_handler(
    gate: _FormatterGate, flush_after_ms: int = _DRAIN_TIMEOUT_MS
) -> _StreamHandler:
    """Build a stderr handler whose formatter blocks until the gate releases."""
    handler = (
        StreamHandlerBuilder
        .stderr()
        .with_flush_after_ms(flush_after_ms)
        .with_formatter(gate.format_record)
        .build()
    )
    return typ.cast("_StreamHandler", handler)


@dataclasses.dataclass(slots=True)
class _DrainOutcome:
    """Observations captured when the native drain call returned."""

    acknowledged: bool | None
    formatter_completed: bool
    waiter_observed: bool
    elapsed_seconds: float
    stderr: str


def _drain_gated_handler(
    capfd: pytest.CaptureFixture[str],
    drain_method: _DrainMethod,
) -> _DrainOutcome:
    """Handle one record, then drain while the formatter is blocked.

    The formatter is released only once the native call reports that it is
    waiting, so a binding that releases the GIL returns as soon as the worker
    finishes, while one that holds the GIL blocks until the handler's flush
    timeout.

    Returns
    -------
    _DrainOutcome
        The observations taken at the moment the drain call returned.
    """
    gate = _FormatterGate()
    handler = _build_gated_handler(gate)
    releaser: threading.Thread | None = None
    acknowledged: bool | None = None
    completed = False
    elapsed = 0.0
    captured = ""
    try:
        handler.handle("logger", "INFO", "hello")
        assert gate.entered.wait(timeout=_GATE_TIMEOUT_SECONDS), (
            "worker never entered the callable formatter"
        )
        releaser = gate.release_when_waiting()
        started = time.monotonic()
        if drain_method == "flush":
            acknowledged = handler.flush()
        else:
            handler.close()
        elapsed = time.monotonic() - started
        completed = gate.completed.is_set()
        captured = capfd.readouterr().err
    finally:
        gate.release.set()
        handler.close()
        if releaser is not None:
            releaser.join(timeout=_GATE_TIMEOUT_SECONDS)
    if gate.waiter_error is not None:
        raise gate.waiter_error
    return _DrainOutcome(
        acknowledged=acknowledged,
        formatter_completed=completed,
        waiter_observed=gate.waiter_observed.is_set(),
        elapsed_seconds=elapsed,
        stderr=captured,
    )


@pytest.mark.parametrize("drain_method", ["flush", "close"])
def test_stream_drain_waits_for_callable_formatter(
    capfd: pytest.CaptureFixture[str],
    drain_method: _DrainMethod,
) -> None:
    """Draining releases the GIL and orders the worker's write."""
    outcome = _drain_gated_handler(capfd, drain_method)
    assert outcome.waiter_observed, f"{drain_method}() never reported a native wait"
    assert outcome.formatter_completed, (
        f"{drain_method}() returned before the formatter completed"
    )
    elapsed = outcome.elapsed_seconds
    assert elapsed < _MAX_DRAIN_SECONDS, (
        f"{drain_method}() waited {elapsed:.3f}s; the GIL was not released"
    )
    assert "gated:hello\n" in outcome.stderr, (
        f"{drain_method}() should order the worker write; stderr={outcome.stderr!r}"
    )
    if drain_method == "flush":
        assert outcome.acknowledged is True, "flush() did not acknowledge the worker"


def test_stream_close_timeout_returns_before_the_worker_writes(
    capfd: pytest.CaptureFixture[str],
) -> None:
    """A close that times out returns while the worker is still writing.

    The formatter stays blocked, so the worker cannot finish. ``close()`` must
    still return close to its configured timeout rather than waiting for the
    worker, and it leaves the record unwritten. Cleanup releases the formatter
    and polls the captured stderr, so the test waits for the worker's actual
    write instead of reading the capture once and racing it.
    """
    gate = _FormatterGate()
    handler = _build_gated_handler(gate, flush_after_ms=_CLOSE_TIMEOUT_MS)
    try:
        handler.handle("logger", "INFO", "hello")
        assert gate.entered.wait(timeout=_GATE_TIMEOUT_SECONDS), (
            "worker never entered the callable formatter"
        )

        started = time.monotonic()
        handler.close()
        elapsed = time.monotonic() - started

        assert elapsed >= _CLOSE_WAIT_FLOOR_SECONDS, (
            f"close() returned after {elapsed:.3f}s; it did not wait out the "
            f"configured {_CLOSE_TIMEOUT_MS}ms timeout"
        )
        assert elapsed < _CLOSE_WAIT_CEILING_SECONDS, (
            f"close() waited {elapsed:.3f}s; the timeout did not bound it"
        )
        assert not gate.completed.is_set(), (
            "close() returned only after the formatter completed; "
            "the timeout path was not exercised"
        )
    finally:
        gate.release.set()
        handler.close()

    poll_captured_text(capfd, "gated:hello\n")
