# ruff: file-ignore[assert] The shared harness asserts on its callers' behalf.
"""Shared harness for the socket handler's close-path integration tests.

The tests in :mod:`tests.test_socket_handler_close` all need the same two
things: a loopback collector that accepts a connection and then never reads
from it, and a handler whose worker is known to be stuck talking to one.
Both live here, so that the test module reads as the assertions it makes.

A stalling endpoint is what makes a close observable. A worker talking to a
healthy collector is answered almost at once, which proves nothing about a
close that has to give up on a worker stuck in a write.
"""

from __future__ import annotations

import socket
import socketserver
import threading
import typing as typ

import femtologging

if typ.TYPE_CHECKING:
    from femtologging import FemtoSocketHandler, SocketHandlerBuilder

# What ``close`` is allowed to take against a worker that never acknowledges.
CLOSE_ALLOWANCE_SECONDS = 3.0
# The handler's flush and shutdown budget.
CLOSE_BUDGET_MS = 200
# How long a worker is kept busy, and so how long an unbounded join waits.
BUSY_WINDOW_SECONDS = 10.0
# How many records are queued behind a worker that cannot reach its collector.
BACKLOG_RECORDS = 16
# How many interpreter polls must land inside a close that holds the GIL. The
# bound is on the interpreter, not the handler: a busy loop in another thread
# either gets scheduled or it does not.
GIL_POLLS_REQUIRED = 10

# A record larger than the receive buffer a stalled peer will ever offer. A
# frame small enough to fit returns from the write at once and never blocks,
# which is the case these tests must not be in. Eight mebibytes is comfortably
# past the largest auto-tuned receive buffer Linux will hand out.
FLOOD_MESSAGE = "x" * (8 << 20)
# The largest frame the handler will send. It has to exceed the serialized size
# of ``FLOOD_MESSAGE``, which carries the message plus the record's own fields.
MAX_FRAME_SIZE = 16 << 20
# The wait for a record to cross a loopback connection, and for the stalled
# peer to have accepted one.
ARRIVAL_ALLOWANCE_SECONDS = 10.0


def recv_exactly(stream: socket.socket, length: int) -> bytes:
    """Read exactly ``length`` bytes, or fewer if the peer closes first."""
    chunks: list[bytes] = []
    remaining = length
    while remaining > 0:
        chunk = stream.recv(remaining)
        if not chunk:
            break
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


class StallingHandler(socketserver.BaseRequestHandler):
    """Accept a connection and never read from it."""

    # Held rather than dropped: closing the connection lets the client's write
    # fail immediately and end the very block this server exists to create.
    connections: typ.ClassVar[list[socket.socket]] = []
    arrived: typ.ClassVar[threading.Event]

    def handle(self) -> None:
        """Record the connection and stall without draining it."""
        self.connections.append(self.request)
        self.arrived.set()


class StallingServer(socketserver.ThreadingTCPServer):
    """Threading TCP server that accepts and stalls."""

    allow_reuse_address = True
    daemon_threads = True

    def __init__(self) -> None:
        super().__init__(("127.0.0.1", 0), StallingHandler)
        self.arrived = threading.Event()


def unused_port() -> int:
    """Reserve an ephemeral port and release it, so nothing is listening."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.bind(("127.0.0.1", 0))
        return int(probe.getsockname()[1])


def backoff_config() -> femtologging.BackoffConfig:
    """Backoff that retries quickly and never expires within a test."""
    return femtologging.BackoffConfig({
        "base_ms": 10,
        "cap_ms": 20,
        "reset_after_ms": 60000,
        "deadline_ms": int(BUSY_WINDOW_SECONDS * 1000),
    })


def build_handler(host: str, port: int, capacity: int) -> SocketHandlerBuilder:
    """Build a socket handler with a short flush budget and a fast backoff."""
    return (
        femtologging
        .SocketHandlerBuilder()
        .with_tcp(host, port)
        .with_capacity(capacity)
        .with_connect_timeout_ms(CLOSE_BUDGET_MS)
        .with_write_timeout_ms(CLOSE_BUDGET_MS)
        .with_max_frame_size(MAX_FRAME_SIZE)
        .with_backoff(backoff_config())
    )


def stall_the_worker(
    server: StallingServer,
    capacity: int = 16,
) -> FemtoSocketHandler:
    """Build a handler whose worker is known to be blocked in a write.

    The flood record is what puts it there: the peer accepts the connection and
    never reads, so the write cannot finish until the handler's own write
    timeout expires. That is the state every close assertion in the test module
    needs, since a close against an idle worker would be answered at once and
    prove nothing.

    Returns
    -------
    FemtoSocketHandler
        A handler whose worker is inside the flood record's write.
    """
    host, port = server.server_address[:2]
    handler = build_handler(str(host), int(port), capacity).build()
    handler.handle("test.logger", "INFO", FLOOD_MESSAGE)
    assert server.arrived.wait(timeout=ARRIVAL_ALLOWANCE_SECONDS), (
        "the worker must reach the endpoint before the close is issued, "
        "otherwise the close is not the path under test"
    )
    return handler


def handle_expecting_refusal(
    handler: FemtoSocketHandler, message: str
) -> RuntimeError | None:
    """Return the error a handler raised for ``message``, or ``None``.

    The caller asserts on the returned error rather than inside the handler,
    so that a refusal is reported with the message that provoked it.

    Returns
    -------
    RuntimeError | None
        The refusal, or ``None`` when the handler accepted the record.
    """
    try:
        handler.handle("test.logger", "INFO", message)
    except RuntimeError as err:
        return err
    return None
