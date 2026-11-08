"""Integration tests for the socket handler's close path, from Python.

The Rust close contracts in ``rust_extension/src/socket_handler/close_tests.rs``
already cover what the handler does to its worker. What they cannot cover is the
interpreter: a ``close`` that holds the GIL while it waits is invisible to Rust
and obvious to Python, which is why the release is asserted here.

The rest of the file exercises the same paths through the Python binding, so
that the binding is known to expose the behaviour rather than only the happy
one: a worker stalled in a write, a worker stuck retrying a connect, a queue
that fills, and a backlog left when the deadline passes.

The stalling collector and the handler builders those tests share live in
:mod:`tests.socket_handler_support`.
"""

from __future__ import annotations

import contextlib
import socketserver
import struct
import threading
import time
import typing as typ

import pytest

from tests.socket_handler_support import (
    ARRIVAL_ALLOWANCE_SECONDS,
    BACKLOG_RECORDS,
    BUSY_WINDOW_SECONDS,
    CLOSE_ALLOWANCE_SECONDS,
    CLOSE_BUDGET_MS,
    GIL_POLLS_REQUIRED,
    StallingServer,
    build_handler,
    handle_expecting_refusal,
    recv_exactly,
    stall_the_worker,
    unused_port,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc


class _GILProbe:
    """A background thread that counts how often it is scheduled.

    A GIL-holding ``close`` or ``flush`` lets other Python threads run only
    before it starts and after it returns. Counting the polls made *during*
    the call is therefore what distinguishes a released interpreter from one
    held for the whole wait; both GIL tests assert the count clears their
    floor.

    The probe sleeps between polls rather than spinning. A tight loop would
    hold the GIL for almost all of its own interval, which both starves the
    thread under test and reports a count that says more about the scheduler
    than about the release.
    """

    def __init__(self) -> None:
        """Start the probe thread, not yet counting."""
        self._polls = 0
        self._is_probing = False
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None

    def __enter__(self) -> _GILProbe:
        """Begin counting polls."""
        self._is_probing = True
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, *_exc: object) -> None:
        """Stop counting and wait for the thread to finish."""
        self._is_probing = False
        self._stop.set()
        if self._thread is not None:
            self._thread.join(timeout=1)

    @property
    def polls(self) -> int:
        """The polls counted so far, including any made after stopping."""
        return self._polls

    def _run(self) -> None:
        """Count polls until the enclosing ``with`` block leaves."""
        while self._is_probing:
            self._polls += 1
            self._stop.wait(timeout=0.001)


@pytest.fixture(name="stalling_server")
def fixture_stalling_server() -> cabc.Iterator[StallingServer]:
    """Serve a loopback TCP endpoint that accepts and never reads."""
    with StallingServer() as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            yield server
        finally:
            server.release.set()
            for connection in server.connections:
                with contextlib.suppress(OSError):
                    connection.close()
            server.connections.clear()
            server.shutdown()
            thread.join(timeout=1)


def test_a_stalled_write_is_reachable_from_python(
    stalling_server: StallingServer,
) -> None:
    """Verify the harness itself: records do reach the stalling endpoint.

    The control for the tests below. Without it a stalling server that refused
    every connection would make them all pass for the wrong reason.
    """
    handler = stall_the_worker(stalling_server)

    handler.close()


def test_close_returns_within_its_budget_against_a_stalled_write(
    stalling_server: StallingServer,
) -> None:
    """A close must be bounded even when the worker never acknowledges.

    The worker is inside a write it cannot finish, so it never reads the
    shutdown command. ``close`` has to return on its own budget rather than
    wait out the worker's backoff deadline, which is ten seconds here against
    200ms of budget: the two figures are far enough apart that a loaded host
    cannot blur them.
    """
    handler = stall_the_worker(stalling_server)

    started = time.monotonic()
    handler.close()
    elapsed = time.monotonic() - started

    assert elapsed < CLOSE_ALLOWANCE_SECONDS, (
        f"close took {elapsed:.3f}s, allowed {CLOSE_ALLOWANCE_SECONDS}s; "
        f"the worker stays busy for {BUSY_WINDOW_SECONDS}s"
    )


def test_close_releases_the_gil_while_it_waits(
    stalling_server: StallingServer,
) -> None:
    """A close that is waiting must let other Python threads run.

    A ``close`` that holds the GIL while it waits for a worker turns a bounded
    wait into a stall of the whole interpreter. The probe thread counts how
    often it is scheduled during the close; a close that held the GIL would let
    it run only before and after, never during.

    The worker is stalled rather than idle on purpose. An idle worker is
    answered in well under a millisecond, which would leave the probe too few
    polls to distinguish "released the GIL" from "never held it".
    """
    handler = stall_the_worker(stalling_server)

    with _GILProbe() as probe:
        started = time.monotonic()
        try:
            handler.close()
        finally:
            elapsed = time.monotonic() - started

    assert elapsed < CLOSE_ALLOWANCE_SECONDS, (
        f"close took {elapsed:.3f}s, allowed {CLOSE_ALLOWANCE_SECONDS}s"
    )
    assert probe.polls >= GIL_POLLS_REQUIRED, (
        f"the probe thread ran only {probe.polls} times across the close; the "
        "interpreter was not available while close was waiting"
    )


def test_close_during_a_connect_backoff_returns_promptly() -> None:
    """A close must interrupt the retry delay, not wait it out.

    Every connect fails here, so the worker spends its time between attempts in
    a backoff delay. A close that could not interrupt one would take the whole
    base delay per attempt, and an unbounded close would take the rest of the
    ten-second backoff deadline; this one is answered by the woken worker.
    """
    handler = build_handler("127.0.0.1", unused_port(), 32).build()
    handler.handle("test.logger", "INFO", "unreachable")
    # Let the worker fail its first connect and enter the delay, so the close
    # lands during one rather than before the first attempt.
    time.sleep(CLOSE_BUDGET_MS / 1000)

    started = time.monotonic()
    handler.close()
    elapsed = time.monotonic() - started

    assert elapsed < CLOSE_ALLOWANCE_SECONDS, (
        f"close took {elapsed:.3f}s against an unreachable collector, allowed "
        f"{CLOSE_ALLOWANCE_SECONDS}s"
    )


def test_a_queue_that_outlasts_the_budget_is_abandoned(
    stalling_server: StallingServer,
) -> None:
    """A close must not work through an unbounded backlog.

    The worker is held in the flood record's stalled write, so the backlog is
    still queued when close arrives. Draining it would cost one write timeout
    per record and exceed the close allowance.
    """
    handler = stall_the_worker(
        stalling_server,
        capacity=BACKLOG_RECORDS * 4,
    )
    for index in range(BACKLOG_RECORDS):
        handler.handle("test.logger", "INFO", f"backlog {index}")

    started = time.monotonic()
    handler.close()
    elapsed = time.monotonic() - started

    assert elapsed < CLOSE_ALLOWANCE_SECONDS, (
        f"close took {elapsed:.3f}s with {BACKLOG_RECORDS} records queued, "
        f"allowed {CLOSE_ALLOWANCE_SECONDS}s"
    )


def test_a_full_queue_reports_the_drop_to_its_caller(
    stalling_server: StallingServer,
) -> None:
    """A full queue must raise, not silently swallow the record.

    The worker is stuck in a write, so it is not reading its command channel
    and a queue of one cannot drain. Filling it is the only way to reach the
    queue-full path from Python, and the caller has to learn that the record
    went nowhere.

    The assertion counts the refusals rather than pinning them to a particular
    iteration: the worker's write timeout keeps elapsing underneath the loop,
    so the queue drains between attempts however briefly. That the queue was
    full at all is the contract; which attempt met it is scheduling.
    """
    handler = stall_the_worker(stalling_server, capacity=1)

    refusals = [
        err
        for index in range(BACKLOG_RECORDS)
        if (err := handle_expecting_refusal(handler, f"overflow {index}")) is not None
    ]

    assert refusals, (
        "the handler accepted every record into a queue of one that the "
        "stalled worker cannot drain; a full queue must be reported"
    )
    for err in refusals:
        assert "queue full" in str(err), (
            f"a refused record must say the queue was full, got {err!r}"
        )

    started = time.monotonic()
    handler.close()
    elapsed = time.monotonic() - started
    assert elapsed < CLOSE_ALLOWANCE_SECONDS, (
        f"close took {elapsed:.3f}s with a full queue, allowed "
        f"{CLOSE_ALLOWANCE_SECONDS}s"
    )


def test_records_delivered_before_the_deadline_still_arrive() -> None:
    """A close must deliver what was queued while the collector was healthy.

    The other half of the discard contract, and the half a close that threw
    everything away would fail. The payload is read back in full rather than
    just its length prefix, because a handler that truncated its last record on
    the way out would leave a short frame behind.
    """
    received: list[bytes] = []
    arrived = threading.Event()

    class _Collector(socketserver.BaseRequestHandler):
        """Collect one framed record so its payload can be read back."""

        def handle(self) -> None:
            """Read one framed record, keeping its payload for the assertion."""
            header = recv_exactly(self.request, 4)
            if len(header) < 4:
                return
            length = int(struct.unpack(">I", header)[0])
            received.append(recv_exactly(self.request, length))
            arrived.set()

    class _CollectorServer(socketserver.ThreadingTCPServer):
        """Serve `_Collector` on loopback for the delivered-record test."""

        allow_reuse_address = True
        daemon_threads = True

    with _CollectorServer(("127.0.0.1", 0), _Collector) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        host, port = server.server_address[:2]
        handler = build_handler(str(host), int(port), 16).build()
        try:
            handler.handle("test.logger", "INFO", "before the close")
            assert arrived.wait(timeout=ARRIVAL_ALLOWANCE_SECONDS), (
                "the record queued before the close must reach the collector"
            )
            handler.close()
        finally:
            server.shutdown()
            thread.join(timeout=1)

    assert len(received) == 1, (
        f"the collector should receive exactly the one record sent, got {len(received)}"
    )
    assert b"before the close" in received[0], (
        "the delivered frame must carry the record it was sent for"
    )


def test_flush_releases_the_gil_while_the_worker_is_blocked(
    stalling_server: StallingServer,
) -> None:
    """A flush must not hold the interpreter while it waits for its worker.

    ``py_flush`` runs the Rust flush inside ``py.detach``, and that release is
    invisible to Rust: only a Python thread can observe whether the interpreter
    stayed available. The probe counts how often it is scheduled during the
    flush, so a flush that held the GIL would let it run only before and after,
    never in between.

    The worker is stalled deliberately. An idle worker acknowledges in well
    under a millisecond, which would leave the probe too few polls to tell
    "released the GIL" from "never held it".

    Kept apart from the close test on purpose: they hold different code paths
    to the same rule, and one failing must say which.
    """
    handler = stall_the_worker(stalling_server)

    with _GILProbe() as probe:
        started = time.monotonic()
        try:
            acknowledged = handler.flush()
        finally:
            elapsed = time.monotonic() - started

    assert acknowledged is False, (
        "a flush against a worker blocked in a write cannot be acknowledged, "
        "so it must report False rather than wait for the stalled worker"
    )
    assert elapsed < CLOSE_ALLOWANCE_SECONDS, (
        f"flush took {elapsed:.3f}s, allowed {CLOSE_ALLOWANCE_SECONDS}s; "
        f"the worker stays busy for {BUSY_WINDOW_SECONDS}s"
    )
    assert probe.polls >= GIL_POLLS_REQUIRED, (
        f"the probe thread ran only {probe.polls} times across the flush; the "
        "interpreter was not available while flush was waiting"
    )
