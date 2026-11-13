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

import sys
import threading
import time
import typing as typ

import pytest

if typ.TYPE_CHECKING:
    from pathlib import Path

from femtologging import FemtoFileHandler, FemtoLogger

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


class BufferingHandler:
    """A user-defined handler that keeps records in its own buffer."""

    def __init__(self) -> None:
        """Initialize the handler with empty received and emitted buffers."""
        self.received: list[str] = []
        self.emitted: list[str] = []

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback ``handle`` shape required by ``FemtoLogger`` validation."""

    def handle_record(self, record: dict[str, object]) -> None:
        """Hold the record in the buffer instead of writing it."""
        self.received.append(str(record["message"]))

    def emit(self) -> None:
        """Write the buffered records, as the application would on its own."""
        self.emitted.extend(self.received)
        self.received.clear()


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

    def test_flush_diagnostics_have_a_stable_schema(self) -> None:
        """Expose fixed-cardinality counters without a logging callback."""
        logger = FemtoLogger("flush_diagnostics")

        before = logger.get_flush_diagnostics()
        assert len(before) == 22, "diagnostics must expose the fixed 22-key schema"
        assert set(before.values()) == {0}, "a new logger must start with zero counters"
        assert before["flush_handlers_attempts"] == 0, "no flush attempt has run yet"
        assert before["flush_handlers_confirmed"] == 0, "no flush is confirmed yet"

        assert logger.flush_handlers(), "an empty logger should flush successfully"

        after = logger.get_flush_diagnostics()
        assert after.keys() == before.keys(), "diagnostic keys must remain stable"
        assert after["flush_handlers_attempts"] == 1, "one flush must add one attempt"
        assert after["flush_handlers_confirmed"] == 1, (
            "the successful flush must be counted"
        )
        assert (
            sum(
                value
                for key, value in after.items()
                if key.startswith("flush_handlers_elapsed_")
            )
            == 1
        ), "the successful attempt must enter exactly one elapsed-time bucket"

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
        flush_entered = threading.Event()

        def note_flush_entry(_frame: object, event: str, arg: object) -> None:
            """Signal the moment the interpreter enters ``flush_handlers``.

            ``flush_handlers`` is a C function, so ``c_call`` is the only
            profile event that reports its entry. A thread that set an event
            just before the call would prove nothing: it could be set while the
            call is still waiting for the interpreter, which is exactly the
            state this test has to rule out.
            """
            if event == "c_call" and getattr(arg, "__name__", "") == "flush_handlers":
                flush_entered.set()

        def flush_and_record() -> None:
            """Enter the barrier, then record the flush outcome."""
            sys.setprofile(note_flush_entry)
            try:
                flushed.append(logger.flush_handlers())
            finally:
                sys.setprofile(None)

        flusher = threading.Thread(target=flush_and_record)
        flusher.start()

        try:
            # The handler is running on the worker, so the worker holds the
            # record. If the flush held the interpreter, this wait could never
            # be satisfied.
            assert handler.handled.wait(HANDSHAKE_SECONDS), (
                "the worker never reached the handler; the flush is holding the "
                "interpreter the worker needs"
            )
            # Release the handler only after the barrier call itself has been
            # entered. Releasing it as soon as the worker arrives would let the
            # worker finish first and the flush would then never contend for
            # the interpreter the handler needs to resume.
            assert flush_entered.wait(HANDSHAKE_SECONDS), (
                "the flush call never entered flush_handlers"
            )
            # Both handshakes were observed while the worker is still blocked
            # inside the handler, so the barrier is still in flight here. A
            # flush that had returned already would have run its worker to
            # completion first, and completion requires this thread to release
            # the handler.
            assert not flushed, (
                f"the flush returned before the handler ran: {flushed!r}"
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

    def test_flush_handlers_drains_a_buffered_file_handler(
        self, tmp_path: Path
    ) -> None:
        """The barrier must flush a Python-registered built-in handler.

        A file handler created through Python is wrapped by ``PyHandler``. If
        the wrapper reported a confirmed flush without asking the wrapped
        handler to flush, records would stay in the handler's buffer while
        ``flush_handlers`` returned ``True``.
        """
        path = tmp_path / "barrier_flush.log"
        handler = FemtoFileHandler(
            str(path),
            capacity=8,
            flush_interval=10000,
            policy="drop",
        )
        try:
            logger = FemtoLogger("buffered_flush")
            logger.add_handler(handler)
            logger.log("INFO", "buffered record")

            assert logger.flush_handlers() is True, (
                "flush_handlers must confirm a barrier it completed"
            )

            assert "buffered record" in path.read_text(encoding="utf-8"), (
                "flush_handlers must drain the file handler's buffer"
            )
        finally:
            handler.close()

    def test_flush_handlers_confirms_dispatch_only_for_a_python_handler(self) -> None:
        """A user-defined Python handler's buffer is outside the barrier.

        The barrier has no flush protocol to invoke on a user-defined Python
        handler, so reaching it counts as confirmation. ``flush_handlers``
        may therefore return ``True`` with the record still in the handler's
        own buffer, which the application flushes itself.
        """
        logger = FemtoLogger("buffering_python")
        handler = BufferingHandler()
        logger.add_handler(handler)
        logger.log("INFO", "buffered by the handler")

        assert logger.flush_handlers() is True, (
            "a dispatched record must count as a confirmed Python flush"
        )
        assert handler.received == ["buffered by the handler"], (
            f"the barrier must wait for dispatch, got {handler.received!r}"
        )
        assert not handler.emitted, (
            "the barrier must not run the application's own buffer flush"
        )

        handler.emit()
        assert handler.emitted == ["buffered by the handler"], (
            "the application's own flush must emit the buffered record"
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
