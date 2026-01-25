"""Multi-threading exception capture tests.

This module validates thread safety of exception capture and payload handling.
It spawns N threads, raises exceptions in each, captures payloads, and asserts:

1. No cross-thread contamination (each thread's payload contains only that
   thread's data).
2. No panics during concurrent exception capture.
3. Payload integrity (correct exception type, message, and stack frames).

The test uses threading.Barrier for deterministic synchronization, avoiding
flaky timing assumptions.

Related to: https://github.com/leynos/femtologging/issues/299
"""

from __future__ import annotations

import re
import threading
import typing as typ

import pytest

from femtologging import FemtoLogger

pytestmark = [pytest.mark.concurrency, pytest.mark.send_sync]


class StackFrameDict(typ.TypedDict):
    """Stack frame structure within exception payloads."""

    filename: str
    lineno: int
    function: str


class ExceptionInfoDict(typ.TypedDict):
    """Exception info payload structure."""

    schema_version: int
    type_name: str
    message: str
    frames: list[StackFrameDict]


class LogRecordDict(typ.TypedDict):
    """Log record structure received by handle_record."""

    logger: str
    level: str
    message: str
    exc_info: typ.NotRequired[ExceptionInfoDict]


class ThreadSpecificError(Exception):
    """Exception for identifying thread-specific exceptions in tests.

    Parameters
    ----------
    thread_index : int
        The index of the thread that raised this exception.

    """

    def __init__(self, thread_index: int) -> None:
        """Initialize with the thread index."""
        self.thread_index = thread_index
        super().__init__(f"Thread {thread_index} exception")


class RecordCollectingHandler:
    """Handler that collects structured log records for validation.

    This handler uses handle_record to receive full structured payloads,
    including exception data, for later test assertions.

    """

    def __init__(self) -> None:
        """Initialize an empty record buffer with thread-safe access."""
        self.records: list[LogRecordDict] = []
        self._lock = threading.Lock()

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback handle method required by FemtoLogger validation."""

    def handle_record(self, record: LogRecordDict) -> None:
        """Collect full records for later assertions."""
        with self._lock:
            self.records.append(record)


def _raise_thread_error(thread_index: int) -> None:
    """Raise a ThreadSpecificError for the given thread index."""
    raise ThreadSpecificError(thread_index)


def _validate_record(record: LogRecordDict, captured_indices: set[int]) -> int:
    """Validate a single log record and return the thread index."""
    expected_message_prefix = "Caught exception in thread "

    # Verify log level is ERROR
    assert record["level"] == "ERROR", f"Unexpected level: {record['level']}"

    # Verify message matches expected format
    message = record["message"]
    assert message.startswith(expected_message_prefix), (
        f"Unexpected message format: {message}"
    )
    thread_idx_from_message = message[len(expected_message_prefix) :]
    assert thread_idx_from_message.isdigit(), (
        f"Thread index not numeric in message: {message}"
    )

    # Verify exc_info is present
    assert "exc_info" in record, f"Record missing exc_info: {record}"
    exc_info = record["exc_info"]

    # Verify exception type
    assert exc_info["type_name"] == "ThreadSpecificError", (
        f"Unexpected exception type: {exc_info['type_name']}"
    )

    # Parse thread index from exception message
    match = re.search(r"Thread (\d+) exception", exc_info["message"])
    assert match is not None, (
        f"Unexpected exception message format: {exc_info['message']}"
    )
    thread_idx = int(match.group(1))

    # Verify log message thread index matches exception payload thread index
    assert int(thread_idx_from_message) == thread_idx, (
        f"Log message thread index ({thread_idx_from_message}) does not match "
        f"exception payload thread index ({thread_idx})"
    )

    # Check for duplicates (would indicate cross-thread contamination)
    assert thread_idx not in captured_indices, (
        f"Duplicate thread index {thread_idx} detected - "
        "possible cross-thread contamination"
    )

    # Verify stack frames contain thread_worker
    assert "frames" in exc_info, f"exc_info missing frames: {exc_info}"
    functions = [f["function"] for f in exc_info["frames"]]
    assert "thread_worker" in functions, (
        f"Stack frames do not contain thread_worker: {functions}"
    )

    return thread_idx


# Barrier timeout in seconds - generous to avoid flaky failures while
# still detecting true deadlocks in a reasonable time frame
BARRIER_TIMEOUT_SECONDS = 30.0


def thread_worker(
    thread_index: int,
    start_barrier: threading.Barrier,
    end_barrier: threading.Barrier,
    logger: FemtoLogger,
) -> None:
    """Execute a worker that raises and logs a thread-specific exception.

    Parameters
    ----------
    thread_index : int
        Unique index identifying this thread.
    start_barrier : threading.Barrier
        Barrier to synchronize thread startup.
    end_barrier : threading.Barrier
        Barrier to synchronize thread completion.
    logger : FemtoLogger
        Logger instance to use for exception capture.

    Raises
    ------
    threading.BrokenBarrierError
        If a barrier wait times out or is broken.

    """
    # Wait for all threads to be ready (timeout prevents permanent hang)
    start_barrier.wait(timeout=BARRIER_TIMEOUT_SECONDS)

    # Raise and capture a thread-specific exception
    try:
        _raise_thread_error(thread_index)
    except ThreadSpecificError:
        logger.log("ERROR", f"Caught exception in thread {thread_index}", exc_info=True)

    # Wait for all threads to complete logging (timeout prevents permanent hang)
    end_barrier.wait(timeout=BARRIER_TIMEOUT_SECONDS)


class TestMultithreadExceptionCapture:
    """Test suite for multi-threaded exception capture validation."""

    @pytest.mark.parametrize("thread_count", [2, 10, 50])
    def test_multithread_exception_capture(  # noqa: PLR6301 - method in class per test guidelines
        self,
        thread_count: int,
    ) -> None:
        """Exception capture should maintain payload integrity across threads.

        This test spawns multiple threads, each raising a unique exception. It
        validates that:

        1. All exceptions are captured (exactly thread_count records).
        2. Each payload contains the correct thread's exception message.
        3. No payload contains data from another thread (no contamination).
        4. Each payload's stack frames include the thread_worker function.
        """
        logger = FemtoLogger("multithread_test")
        handler = RecordCollectingHandler()
        logger.add_handler(handler)

        # Barriers include main thread (+1) to ensure we don't validate early
        start_barrier = threading.Barrier(thread_count + 1)
        end_barrier = threading.Barrier(thread_count + 1)

        # Spawn worker threads
        threads = [
            threading.Thread(
                target=thread_worker,
                args=(i, start_barrier, end_barrier, logger),
            )
            for i in range(thread_count)
        ]

        for t in threads:
            t.start()

        # Release all threads to start together (timeout prevents permanent hang)
        try:
            start_barrier.wait(timeout=BARRIER_TIMEOUT_SECONDS)
        except threading.BrokenBarrierError as exc:
            pytest.fail(f"Start barrier timed out or was broken: {exc}")

        # Wait for all threads to complete logging (timeout prevents permanent hang)
        try:
            end_barrier.wait(timeout=BARRIER_TIMEOUT_SECONDS)
        except threading.BrokenBarrierError as exc:
            pytest.fail(f"End barrier timed out or was broken: {exc}")

        for t in threads:
            t.join(timeout=BARRIER_TIMEOUT_SECONDS)
            if t.is_alive():
                pytest.fail(f"Thread {t.name} failed to join within timeout")

        # Flush handlers and delete logger to ensure all records are processed
        # flush_handlers() triggers the worker to process pending records,
        # and deleting the logger ensures the worker thread drains completely
        logger.flush_handlers()
        del logger

        # Validate captured records
        assert len(handler.records) == thread_count, (
            f"Expected {thread_count} records, got {len(handler.records)}"
        )

        # Extract and validate thread indices from captured records
        captured_indices: set[int] = set()
        for record in handler.records:
            thread_idx = _validate_record(record, captured_indices)
            captured_indices.add(thread_idx)

        # All thread indices should be accounted for
        expected_indices = set(range(thread_count))
        assert captured_indices == expected_indices, (
            f"Missing thread indices: {expected_indices - captured_indices}, "
            f"unexpected indices: {captured_indices - expected_indices}"
        )
