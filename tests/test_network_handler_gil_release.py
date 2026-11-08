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
# Enough data to overflow loopback TCP buffers, so the socket worker blocks in
# its write rather than finishing before the wait under test begins.
LARGE_MESSAGE = "x" * (256 * 1024)
LARGE_MESSAGE_COUNT = 32


@pytest.fixture
def silent_server() -> cabc.Iterator[int]:
    """Yield the port of a server that accepts connections and never reads."""
    connections: list[socket.socket] = []
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

        acceptor = threading.Thread(target=accept_forever, daemon=True)
        acceptor.start()
        yield listener.getsockname()[1]
        is_stopping.set()
        acceptor.join()
    for connection in connections:
        connection.close()


def _ticks_during(call: cabc.Callable[[], object]) -> tuple[int, float]:
    """Return how often another thread ran during ``call``, and its duration."""
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
    ticks_before = ticks[0]
    started = time.monotonic()
    call()
    elapsed = time.monotonic() - started
    ticks_during = ticks[0] - ticks_before
    is_finished.set()
    ticker.join()
    return ticks_during, elapsed


def _stalled_socket_handler(port: int) -> _NetworkHandler:
    """Build a socket handler whose worker is blocked writing to ``port``."""
    handler = (
        SocketHandlerBuilder()
        .with_tcp("127.0.0.1", port)
        .with_write_timeout_ms(WRITE_TIMEOUT_MS)
        .build()
    )
    for _ in range(LARGE_MESSAGE_COUNT):
        handler.handle("app", "INFO", LARGE_MESSAGE)
    return typ.cast("_NetworkHandler", handler)


def _stalled_http_handler(port: int) -> _NetworkHandler:
    """Build an HTTP handler whose worker awaits a response from ``port``."""
    handler = (
        HTTPHandlerBuilder()
        .with_endpoint(f"http://127.0.0.1:{port}/ingest")
        .with_write_timeout_ms(WRITE_TIMEOUT_MS)
        .build()
    )
    handler.handle("app", "INFO", "never answered")
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
    build_handler: cabc.Callable[[int], _NetworkHandler],
    method: str,
    silent_server: int,
) -> None:
    """Other Python threads keep running while the handler waits on its worker."""
    handler = build_handler(silent_server)
    try:
        ticks, elapsed = _ticks_during(getattr(handler, method))
    finally:
        handler.close()
    assert elapsed >= MINIMUM_WAIT_SECONDS, (
        f"{method}() returned after {elapsed:.3f}s, so the worker never stalled "
        "and this test proved nothing"
    )
    assert ticks >= MINIMUM_TICKS, (
        f"another thread ran only {ticks} times during {elapsed:.3f}s of "
        f"{method}(), so the wait held the GIL"
    )
