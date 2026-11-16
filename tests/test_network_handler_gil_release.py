"""Regression tests: network handler waits must not freeze other Python threads.

A socket or HTTP worker stuck on an unresponsive peer can keep ``flush()`` or
``close()`` waiting for as long as the write timeout. Python-facing waits must
release the GIL, or every other Python thread stops for that whole period.
"""

from __future__ import annotations

import socket
import sys
import threading
import time
import typing as typ

import pytest

from femtologging import HTTPHandlerBuilder, SocketHandlerBuilder

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
# The interpreter's own GIL switch interval, captured so a measured call can
# put it back. ``UNPREEMPTED_SWITCH_SECONDS`` is long enough that no thread can
# ask for the GIL inside a measured call; see ``_ticks_during``.
DEFAULT_SWITCH_SECONDS = sys.getswitchinterval()
UNPREEMPTED_SWITCH_SECONDS = 60.0
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


def _ticks_during(
    prepare: cabc.Callable[[], cabc.Callable[[], object]],
) -> tuple[int, float]:
    """Measure another thread's progress during the wait ``prepare`` sets up.

    ``prepare`` builds the handler and queues its work once the ticker is
    already running, and hands back the wait to measure. Setting up inside the
    timed region would let the setup hide the very stall under test.

    Returns
    -------
    The number of ticks another thread completed during the measured call, and
    the call's duration in seconds.
    """
    ticks = [0]
    has_ticked = threading.Event()
    is_finished = threading.Event()

    def tick() -> None:
        """Count how often this thread gets to run."""
        while not is_finished.is_set():
            ticks[0] += 1
            has_ticked.set()
            time.sleep(TICK_SECONDS)

    ticker = threading.Thread(target=tick, daemon=True)
    ticker.start()
    assert has_ticked.wait(timeout=5), "the ticker thread never started"
    try:
        call = prepare()
        # The interpreter hands the GIL to a waiting thread on request, so the
        # caller can be preempted between reading the clock and entering the
        # call. A pause there counts as call time, which would let a wait that
        # held the GIL pass, and can also shrink a correct wait below the
        # minimum. Raising the switch interval and giving the GIL up once
        # drains any handover already requested and restarts the interval, so
        # the caller reaches the call uninterrupted. Only a voluntary release
        # inside the call can then let the ticker run, which is what the count
        # proves.
        sys.setswitchinterval(UNPREEMPTED_SWITCH_SECONDS)
        time.sleep(0)
        ticks_before = ticks[0]
        started = time.monotonic()
        try:
            call()
        finally:
            elapsed = time.monotonic() - started
            sys.setswitchinterval(DEFAULT_SWITCH_SECONDS)
        return ticks[0] - ticks_before, elapsed
    finally:
        is_finished.set()
        ticker.join()


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
        ticks, elapsed = _ticks_during(prepare)
    finally:
        for handler in built:
            handler.close()
    assert elapsed >= MINIMUM_WAIT_SECONDS, (
        f"{method}() returned after {elapsed:.3f}s, so the worker never stalled "
        "and this test proved nothing"
    )
    assert ticks >= MINIMUM_TICKS, (
        f"another thread ran only {ticks} times during {elapsed:.3f}s of "
        f"{method}(), so the wait held the GIL"
    )
