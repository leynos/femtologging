"""Integration tests for the FemtoHTTPHandler Python surface.

The Rust tests already cover the shutdown contract against a server that never
answers. What they cannot cover is the interpreter: a ``close`` that holds the
GIL while it waits is invisible to Rust and obvious to Python, which is why it
is asserted here.
"""

from __future__ import annotations

import contextlib
import http.server
import threading
import time
import typing as typ

import pytest

import femtologging

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from femtologging import FemtoHTTPHandler

# What ``close`` is allowed to take against a server that never answers.
CLOSE_ALLOWANCE_SECONDS = 3.0
# The handler's flush and shutdown budget.
CLOSE_BUDGET_MS = 200
# How long the worker is kept busy, and so how long an unbounded join waits.
BUSY_WINDOW_SECONDS = 10.0
# How many interpreter polls must land inside a close that is holding the GIL.
# The bound is on the interpreter, not the handler: a busy loop in another
# thread either gets scheduled or it does not.
GIL_POLLS_REQUIRED = 10


class _StallingHandler(http.server.BaseHTTPRequestHandler):
    """Accept requests and never answer them."""

    # Held rather than dropped: closing the connection lets the client fail
    # immediately and end the very block this server exists to create.
    connections: typ.ClassVar[list[typ.Any]] = []

    def do_POST(self) -> None:
        """Read the request and stall without replying."""
        length = int(self.headers.get("Content-Length", "0"))
        self.rfile.read(length)
        self.connections.append(self.connection)
        server = typ.cast("_StallingServer", self.server)
        server.arrived.set()

    def log_message(
        self,
        format: str,  # ruff: ignore[builtin-argument-shadowing] stdlib override requires the parameter name
        *arguments: object,
    ) -> None:
        """Silence the default stderr logging."""
        _ = (format, arguments)


class _StallingServer(http.server.ThreadingHTTPServer):
    """Threading HTTP server that stalls every request it accepts."""

    allow_reuse_address = True
    daemon_threads = True

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), _StallingHandler)
        self.arrived = threading.Event()


@pytest.fixture(name="stalling_server")
def fixture_stalling_server() -> cabc.Iterator[_StallingServer]:
    """Serve a loopback HTTP endpoint that accepts and never answers."""
    with _StallingServer() as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield server
        finally:
            for connection in _StallingHandler.connections:
                with contextlib.suppress(OSError):
                    connection.close()
            _StallingHandler.connections.clear()
            server.shutdown()
            thread.join(timeout=1)


def _build_stalling_handler(server: _StallingServer) -> FemtoHTTPHandler:
    """Build a handler pointed at the stalling server."""
    host, port = server.server_address[:2]
    return (
        femtologging
        .HTTPHandlerBuilder()
        .with_endpoint(f"http://{host}:{port}/log", "POST")
        .with_connect_timeout_ms(CLOSE_BUDGET_MS)
        .with_write_timeout_ms(CLOSE_BUDGET_MS)
        .with_backoff(
            femtologging.BackoffConfig({
                "base_ms": 10,
                "cap_ms": 20,
                "reset_after_ms": 60000,
                "deadline_ms": int(BUSY_WINDOW_SECONDS * 1000),
            })
        )
        .build()
    )


def test_http_handler_sends_records(
    stalling_server: _StallingServer,
) -> None:
    """Verify the handler reaches the endpoint and reports a close."""
    handler = _build_stalling_handler(stalling_server)
    handler.handle("test.logger", "INFO", "message")

    assert stalling_server.arrived.wait(timeout=10), (
        "the handler must send the queued record to the endpoint"
    )
    handler.close()


def test_close_returns_within_its_budget_against_a_stalled_request(
    stalling_server: _StallingServer,
) -> None:
    """A close must be bounded even when the worker never acknowledges.

    The worker is inside a request it cannot finish, so it never reads the
    shutdown command. ``close`` has to return on its own budget rather than
    wait out the worker's backoff deadline, which is ten seconds here and 200ms
    of budget: the two figures are far enough apart that a loaded host cannot
    blur them.
    """
    handler = _build_stalling_handler(stalling_server)
    handler.handle("test.logger", "INFO", "stalling")
    assert stalling_server.arrived.wait(timeout=10), (
        "the worker must reach the endpoint before the close is issued, "
        "otherwise the close is not the path under test"
    )

    started = time.monotonic()
    handler.close()
    elapsed = time.monotonic() - started

    assert elapsed < CLOSE_ALLOWANCE_SECONDS, (
        f"close took {elapsed:.3f}s, allowed {CLOSE_ALLOWANCE_SECONDS}s; "
        f"the worker stays busy for {BUSY_WINDOW_SECONDS}s"
    )


def test_close_releases_the_gil_while_it_waits(
    stalling_server: _StallingServer,
) -> None:
    """A close that is waiting must let other Python threads run.

    A ``close`` that holds the GIL while it waits for a worker turns a bounded
    wait into a stall of the whole interpreter. The probe thread counts how
    often it is scheduled during the close; a close that held the GIL would
    let it run only before and after, never during.
    """
    handler = _build_stalling_handler(stalling_server)
    handler.handle("test.logger", "INFO", "gil")
    assert stalling_server.arrived.wait(timeout=10), (
        "the worker must reach the endpoint before the close is issued"
    )

    polls = 0
    closed = threading.Event()
    probing = True

    def probe() -> None:
        nonlocal polls
        while probing:
            polls += 1
            time.sleep(0.001)

    def close_handler() -> None:
        handler.close()
        closed.set()

    probe_thread = threading.Thread(target=probe, daemon=True)
    probe_thread.start()
    closer = threading.Thread(target=close_handler, daemon=True)
    closer.start()
    try:
        assert closed.wait(timeout=CLOSE_ALLOWANCE_SECONDS * 4), (
            "close must return within its allowance"
        )
    finally:
        probing = False
        closer.join(timeout=1)
        probe_thread.join(timeout=1)

    assert polls >= GIL_POLLS_REQUIRED, (
        f"the probe thread ran only {polls} times across the close; the "
        "interpreter was not available while close was waiting"
    )
