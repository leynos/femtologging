"""Regression tests for the flush barrier in :meth:`FemtoLogger.flush_handlers`.

``flush_handlers`` waits for the logger's worker to reach a marker record
queued behind every record the logger has already accepted. That wait is the
one place the logger blocks on its own worker, so it is the one place the
Python handler interface can deadlock: the worker needs the interpreter to
call a handler, and a caller that holds the interpreter while waiting for the
worker holds the only thing that would let it finish.

These tests pin both halves of the contract that closes that deadlock: the
wait releases the interpreter, so a Python handler continues to run while
another thread flushes; and the wait is bounded, so a flush that cannot reach
its marker returns ``False`` instead of blocking forever.
"""

from __future__ import annotations

import threading
import time

import pytest

from femtologging import FemtoLogger

pytestmark = [pytest.mark.concurrency]

# The timeout the logger advertises for the whole barrier, in seconds, and the
# multiple of it a test may take before the call counts as unbounded. The
# defect under test is waiting forever, so a generous allowance still
# distinguishes a bounded wait from an unbounded one on a loaded runner.
FLUSH_TIMEOUT_SECONDS = 2.0
FLUSH_BOUND_SECONDS = 4.0

# Bounded wait for a cross-thread handshake, short enough that a hang fails
# the test rather than the suite's global timeout.
HANDSHAKE_SECONDS = 20.0


class GilNeedingHandler:
    """A handler whose ``handle`` only completes while it holds the interpreter."""

    def __init__(self) -> None:
        """Initialize the handler with its handshake primitives."""
        self.handled = threading.Event()
        self.proceed = threading.Event()

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback ``handle`` shape required by ``FemtoLogger`` validation."""

    def handle_record(self, _record: dict[str, object]) -> None:
        """Run on the logger's worker, which must hold the interpreter here."""
        self.handled.set()
        if not self.proceed.wait(HANDSHAKE_SECONDS):
            msg = "the test never released the handler"
            raise AssertionError(msg)


class StallingHandler:
    """A handler that blocks the worker indefinitely."""

    def __init__(self, entering: threading.Event) -> None:
        """Initialize the handler, signalling on *entering* when it blocks."""
        self.entering = entering
        self.release = threading.Event()

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback ``handle`` shape required by ``FemtoLogger`` validation."""

    def handle_record(self, _record: dict[str, object]) -> None:
        """Block the worker until *release* is set."""
        self.entering.set()
        self.release.wait(HANDSHAKE_SECONDS)


class TestFlushHandlersBarrier:
    """Contracts for the logger's cross-thread flush barrier."""

    def test_flush_handlers_does_not_deadlock_on_a_gil_needing_handler(self) -> None:
        """A flush must not hold the interpreter while its worker needs it.

        The handler blocks inside ``handle`` until the test releases it, so the
        flush can only reach its marker once the handler has run. With the
        interpreter held across the wait, the worker cannot run the handler and
        the flush cannot finish; with the interpreter released, both proceed.
        """
        logger = FemtoLogger("gil_flush")
        handler = GilNeedingHandler()
        logger.add_handler(handler)
        logger.log("INFO", "blocking record")

        flushed: list[bool] = []
        flusher = threading.Thread(
            target=lambda: flushed.append(logger.flush_handlers())
        )
        flusher.start()

        # The barrier is reached, not merely queued: the handler is running, so
        # the worker has the record. If the flush held the interpreter, this wait
        # would never be satisfied.
        try:
            assert handler.handled.wait(HANDSHAKE_SECONDS), (
                "the worker never reached the handler; the flush is holding the "
                "interpreter the worker needs"
            )
        finally:
            handler.proceed.set()
            flusher.join(HANDSHAKE_SECONDS)

        assert not flusher.is_alive(), (
            "flush_handlers did not return after the handler ran"
        )
        assert flushed == [True], (
            f"a flush with a live worker should succeed, got {flushed!r}"
        )

    def test_flush_handlers_is_bounded_when_a_handler_stalls(self) -> None:
        """A stalled handler must produce ``False`` within the advertised timeout.

        The queue is left with room, so the marker is enqueued and the failure is
        the acknowledgement timing out — the path a full queue's ``send`` used to
        bypass entirely, because the marker was never enqueued at all.
        """
        logger = FemtoLogger("stalled_flush")
        entering = threading.Event()
        handler = StallingHandler(entering)
        logger.add_handler(handler)
        logger.log("INFO", "stalling record")

        try:
            assert entering.wait(HANDSHAKE_SECONDS), (
                "the worker never reached the handler"
            )

            started_at = time.monotonic()
            flushed = logger.flush_handlers()
            elapsed = time.monotonic() - started_at

            assert flushed is False, (
                "a flush the worker never acknowledges must be False"
            )
            assert elapsed < FLUSH_BOUND_SECONDS, (
                f"flush_handlers took {elapsed:.2f}s with a stalled handler; the "
                f"advertised timeout is {FLUSH_TIMEOUT_SECONDS}s"
            )
        finally:
            handler.release.set()
