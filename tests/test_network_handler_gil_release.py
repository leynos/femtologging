"""Regression tests: network handler waits must not freeze other Python threads.

A socket or HTTP worker stuck on an unresponsive peer can keep ``flush()`` or
``close()`` waiting for as long as the write timeout. Python-facing waits must
release the GIL, or every other Python thread stops for that whole period.
"""

from __future__ import annotations

import socket
import threading
import time
import typing as typ

import pytest

from femtologging import HTTPHandlerBuilder, SocketHandlerBuilder
from femtologging import _femtologging_rs as rust

# Both hooks exist only when the extension was built with the Rust `test-util`
# feature, which the test builds enable.
_drain_gil_release_signals = getattr(rust, "drain_gil_release_signals_for_test", None)
_wait_for_gil_release = getattr(rust, "wait_for_gil_release_for_test", None)
GIL_RELEASE_SIGNAL_AVAILABLE = callable(_drain_gil_release_signals) and callable(
    _wait_for_gil_release
)

pytestmark = pytest.mark.skipif(
    not GIL_RELEASE_SIGNAL_AVAILABLE,
    reason="requires Rust extension built with the 'test-util' feature",
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    class _NetworkHandler(typ.Protocol):
        """The handler surface these tests drive."""

        def handle(self, logger: str, level: str, message: str) -> None:
            """Queue one record for the handler's worker."""

        def flush(self) -> bool:
            """Wait for the worker to flush queued records."""

        def close(self) -> None:
            """Stop the worker and wait for it to finish."""


WRITE_TIMEOUT_MS = 300
# Well below the write timeout, so a released GIL always clears these bars.
MINIMUM_WAIT_SECONDS = 0.15
MINIMUM_TICKS = 5
TICK_SECONDS = 0.005
# How long each wait for the call's GIL-release signal blocks before the
# observer checks whether the call has finished.
WAIT_ENTRY_POLL_MS = 50
# Enough data to overflow loopback TCP buffers, so the socket worker blocks in
# its write rather than finishing before the wait under test begins.
LARGE_MESSAGE = "x" * (256 * 1024)
LARGE_MESSAGE_COUNT = 32


class _SilentServer(typ.NamedTuple):
    """A server that accepts connections and never answers them."""

    port: int
    connected: threading.Event
    """Set once a handler has connected, so a test can wait for that."""


@pytest.fixture
def silent_server() -> cabc.Iterator[_SilentServer]:
    """Yield a server that accepts connections and never reads or answers."""
    connections: list[socket.socket] = []
    connected = threading.Event()
    with socket.create_server(("127.0.0.1", 0)) as listener:
        listener.settimeout(0.05)
        is_stopping = threading.Event()

        def accept_forever() -> None:
            """Accept connections until the fixture stops."""
            while not is_stopping.is_set():
                try:
                    connections.append(listener.accept()[0])
                except TimeoutError:
                    continue
                connected.set()

        acceptor = threading.Thread(target=accept_forever, daemon=True)
        acceptor.start()
        yield _SilentServer(listener.getsockname()[1], connected)
        is_stopping.set()
        acceptor.join()
    for connection in connections:
        connection.close()


class _WaitMeasurement(typ.NamedTuple):
    """What other threads observed while a measured wait was running."""

    ticks: int
    """Ticks counted from the moment the call reported releasing the GIL."""

    elapsed: float
    """How long the measured call took, in seconds."""


class _Ticker:
    """Count how often its thread gets to run."""

    def __init__(self) -> None:
        """Start with nothing counted and a thread that is not yet running."""
        self.ticks = 0
        self._is_finished = threading.Event()
        self._has_ticked = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        """Start counting, and wait until the counter has moved once."""
        self._thread.start()
        assert self._has_ticked.wait(timeout=5), "the ticker thread never started"

    def stop(self) -> None:
        """Stop counting once the thread has finished."""
        self._is_finished.set()
        self._thread.join()

    def _run(self) -> None:
        """Increment the count, giving the GIL up between increments."""
        while not self._is_finished.is_set():
            self.ticks += 1
            self._has_ticked.set()
            time.sleep(TICK_SECONDS)


class _WaitEntryObserver:
    """Record the tick count as soon as the measured call releases the GIL."""

    def __init__(self, ticker: _Ticker, is_call_finished: threading.Event) -> None:
        """Watch for the call's own GIL-release signal."""
        self.baseline: int | None = None
        self._ticker = ticker
        self._is_call_finished = is_call_finished

    def watch_until_finished(self) -> None:
        """Take the baseline once the call reports that it released the GIL.

        The extension raises that signal as the last thing it does while still
        holding the GIL, immediately before detaching. Consuming it therefore
        proves the call has given the interpreter up: a wait that held the GIL
        would never let this thread run, so its signal could not be received
        until after the call had already returned.
        """
        wait_for_release = _wait_for_gil_release
        assert wait_for_release is not None, "the test-util hooks are missing"
        while not self._is_call_finished.is_set():
            if wait_for_release(WAIT_ENTRY_POLL_MS):
                self.baseline = self._ticker.ticks
                return


def _ticks_during(
    prepare: cabc.Callable[[], cabc.Callable[[], object]],
) -> _WaitMeasurement:
    """Measure another thread's progress during the wait ``prepare`` sets up.

    ``prepare`` builds the handler and queues its work once the ticker is
    already running, and hands back the wait to measure. Setting up inside the
    timed region would let the setup hide the very stall under test.

    The count starts when an observer receives the call's GIL-release signal,
    which the extension raises immediately before detaching. Ticks before that
    signal, and ticks after the call returns while the caller holds the GIL
    again, cannot be counted, so neither a slow schedule nor a GIL-holding wait
    can produce a false pass.

    Returns
    -------
    The ticks another thread completed while the call waited with the GIL
    released, and how long the call took.
    """
    ticker = _Ticker()
    is_call_finished = threading.Event()
    observer = _WaitEntryObserver(ticker, is_call_finished)
    ticker.start()
    try:
        call = prepare()
        # Arm the gate on a clean channel: a signal left by an earlier call
        # belongs to that call, not this one.
        drain_signals = _drain_gil_release_signals
        assert drain_signals is not None, "the test-util hooks are missing"
        drain_signals()
        watcher = threading.Thread(
            target=observer.watch_until_finished,
            daemon=True,
        )
        watcher.start()
        started = time.monotonic()
        try:
            call()
        finally:
            elapsed = time.monotonic() - started
            # Read the count before releasing the GIL, so no tick after the
            # call can be mistaken for one the call let through.
            baseline = observer.baseline
            counted = 0 if baseline is None else ticker.ticks - baseline
            is_call_finished.set()
            watcher.join(timeout=5)
        return _WaitMeasurement(counted, elapsed)
    finally:
        ticker.stop()


def _stalled_socket_handler(server: _SilentServer) -> _NetworkHandler:
    """Build a socket handler whose worker is blocked writing to ``server``."""
    handler = (
        SocketHandlerBuilder()
        .with_tcp("127.0.0.1", server.port)
        .with_write_timeout_ms(WRITE_TIMEOUT_MS)
        .build()
    )
    for _ in range(LARGE_MESSAGE_COUNT):
        handler.handle("app", "INFO", LARGE_MESSAGE)
    # Wait for the worker to connect before the caller starts timing, so the
    # measured call cannot arrive after the worker has finished with the peer.
    assert server.connected.wait(timeout=5), "the socket worker never connected"
    return typ.cast("_NetworkHandler", handler)


def _stalled_http_handler(server: _SilentServer) -> _NetworkHandler:
    """Build an HTTP handler whose worker awaits a response from ``server``."""
    handler = (
        HTTPHandlerBuilder()
        .with_endpoint(f"http://127.0.0.1:{server.port}/ingest")
        .with_write_timeout_ms(WRITE_TIMEOUT_MS)
        .build()
    )
    handler.handle("app", "INFO", "never answered")
    # The request timeout starts when the worker sends, so measuring from the
    # connection leaves the whole timeout inside the call being measured.
    assert server.connected.wait(timeout=5), "the HTTP worker never connected"
    return typ.cast("_NetworkHandler", handler)


@pytest.mark.parametrize(
    ("build_handler", "method"),
    [
        pytest.param(_stalled_socket_handler, "flush", id="socket-flush"),
        pytest.param(_stalled_socket_handler, "close", id="socket-close"),
        pytest.param(_stalled_http_handler, "close", id="http-close"),
    ],
)
def test_wait_on_a_stalled_worker_lets_python_threads_run(
    build_handler: cabc.Callable[[_SilentServer], _NetworkHandler],
    method: str,
    silent_server: _SilentServer,
) -> None:
    """Other Python threads keep running while the handler waits on its worker."""
    built: list[_NetworkHandler] = []

    def prepare() -> cabc.Callable[[], object]:
        """Build and block the handler's worker before the timed call begins."""
        handler = build_handler(silent_server)
        built.append(handler)
        return getattr(handler, method)

    try:
        measurement = _ticks_during(prepare)
    finally:
        for handler in built:
            handler.close()
    assert measurement.elapsed >= MINIMUM_WAIT_SECONDS, (
        f"{method}() returned after {measurement.elapsed:.3f}s, so the worker "
        "never stalled and this test proved nothing"
    )
    assert measurement.ticks >= MINIMUM_TICKS, (
        f"another thread ran only {measurement.ticks} times during "
        f"{measurement.elapsed:.3f}s of {method}(), so the wait held the GIL"
    )
