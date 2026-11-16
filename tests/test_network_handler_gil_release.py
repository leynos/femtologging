"""Regression tests: network handler waits must not freeze other Python threads.

A socket or HTTP worker stuck on an unresponsive peer can keep ``flush()`` or
``close()`` waiting for as long as the write timeout. Python-facing waits must
release the GIL, or every other Python thread stops for that whole period.
"""

from __future__ import annotations

import dis
import socket
import sys
import threading
import time
import typing as typ

import pytest

from femtologging import HTTPHandlerBuilder, SocketHandlerBuilder

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    import types

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
    """Ticks counted from the moment another thread saw the call running."""

    saw_call: bool
    """Whether any other thread ever saw the call on the calling thread."""

    elapsed: float
    """How long the measured call took, in seconds."""


def _invoke(call: cabc.Callable[[], object]) -> object:
    """Run ``call`` from a frame the observer thread can recognise.

    The only statement is the call itself, so this frame sits on the calling
    thread's stack exactly while the call runs. The interpreter still reports
    it while the call waits with its thread state detached, which is what lets
    another thread prove the call has started.

    Returns
    -------
    Whatever ``call`` returns.
    """
    return call()


# Byte offsets of the call instructions in ``_invoke``. A frame reports the
# instruction it was last executing, so requiring one of these offsets places
# the observation inside the call. The frame alone would not: a thread can be
# switched out as soon as the frame exists, before the call runs, and an
# observer that accepted that would measure whatever pause came first.
CALL_OFFSETS = frozenset(
    instruction.offset
    for instruction in dis.get_instructions(_invoke)
    if instruction.opname.startswith("CALL")
)
# The call is the helper's only statement, so its only call instruction is the
# one under test. A second one would let the observer accept the helper while it
# ran something else, which is the window this guards against.
assert len(CALL_OFFSETS) == 1, "_invoke must make exactly one call"


def _is_at_call(frame: types.FrameType) -> bool:
    """Report whether ``frame`` is running ``_invoke``'s call instruction."""
    return frame.f_code is _invoke.__code__ and frame.f_lasti in CALL_OFFSETS


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


class _CallObserver:
    """Record the tick count as soon as the measured call is seen running."""

    def __init__(
        self,
        caller: int,
        ticker: _Ticker,
        is_call_finished: threading.Event,
    ) -> None:
        """Watch ``caller``'s stack and report ``ticker``'s count on entry."""
        self.baseline: int | None = None
        self._caller = caller
        self._ticker = ticker
        self._is_call_finished = is_call_finished

    def watch_until_finished(self) -> None:
        """Take the baseline once the call itself is running on the caller.

        Both the frame and its instruction offset have to match. The frame
        alone proves only that ``_invoke`` was entered, which happens before
        the call executes; the call instruction is what the caller is running
        when the wait under test is in progress.
        """
        while not self._is_call_finished.is_set():
            frame = sys._current_frames().get(self._caller)
            if frame is not None and _is_at_call(frame):
                self.baseline = self._ticker.ticks
                return
            time.sleep(TICK_SECONDS)


def _ticks_during(
    prepare: cabc.Callable[[], cabc.Callable[[], object]],
) -> _WaitMeasurement:
    """Measure another thread's progress during the wait ``prepare`` sets up.

    ``prepare`` builds the handler and queues its work once the ticker is
    already running, and hands back the wait to measure. Setting up inside the
    timed region would let the setup hide the very stall under test.

    The count starts when an observer thread first sees the call instruction
    running on this thread, not before the call. Time before the call cannot
    therefore be counted as call time, and a call that holds the GIL lets no
    tick through after that observation, so neither a slow schedule nor a
    GIL-holding wait can produce a false pass.

    Returns
    -------
    The ticks another thread completed while the call was seen running, whether
    it was ever seen, and how long the call took.
    """
    ticker = _Ticker()
    is_call_finished = threading.Event()
    observer = _CallObserver(threading.get_ident(), ticker, is_call_finished)
    ticker.start()
    try:
        call = prepare()
        watcher = threading.Thread(
            target=observer.watch_until_finished,
            daemon=True,
        )
        watcher.start()
        started = time.monotonic()
        try:
            _invoke(call)
        finally:
            elapsed = time.monotonic() - started
            # Read the count before releasing the GIL, so no tick after the
            # call can be mistaken for one the call let through.
            baseline = observer.baseline
            counted = None if baseline is None else ticker.ticks - baseline
            is_call_finished.set()
            watcher.join(timeout=5)
        return _WaitMeasurement(
            0 if counted is None else counted,
            counted is not None,
            elapsed,
        )
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
    assert measurement.saw_call, (
        f"no other thread ever saw {method}() running, so the wait held the GIL"
    )
    assert measurement.ticks >= MINIMUM_TICKS, (
        f"another thread ran only {measurement.ticks} times during "
        f"{measurement.elapsed:.3f}s of {method}(), so the wait held the GIL"
    )
