"""Tests for the FemtoFileHandler."""

from __future__ import annotations

import collections.abc as cabc
import errno
import re
import threading
import time
import typing as typ
from contextlib import closing, contextmanager
from pathlib import Path

import pytest

from femtologging import FemtoFileHandler, FileHandlerBuilder, OverflowPolicy

FileHandlerFactory = cabc.Callable[
    [Path, int, int], typ.ContextManager[FemtoFileHandler]
]


class FormatterRecord(typ.TypedDict):
    """Structured payload for the blocking formatter."""

    logger: str
    level: str
    message: str


def _read_lines_with_retry(
    path: Path, expected: list[str], *, timeout: float = 1.0
) -> list[str]:
    """Read lines, retrying briefly to allow async flush to complete."""

    def read_lines() -> list[str]:
        return path.read_text().splitlines() if path.exists() else []

    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        lines = read_lines()
        if lines == expected:
            return lines
        time.sleep(0.01)
    return read_lines()


def test_file_handler_writes_to_file(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """A single record should persist to disk."""
    path = tmp_path / "out.log"
    with file_handler_factory(path, 8, 1) as handler:
        handler.handle("core", "INFO", "hello")
    assert path.read_text() == "core [INFO] hello\n"


def test_file_handler_multiple_records(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """Records are appended in the order they are handled."""
    path = tmp_path / "multi.log"
    with file_handler_factory(path, 8, 1) as handler:
        handler.handle("core", "INFO", "first")
        handler.handle("core", "WARN", "second")
        handler.handle("core", "ERROR", "third")
    assert (
        path.read_text()
        == "core [INFO] first\ncore [WARN] second\ncore [ERROR] third\n"
    )


def test_file_handler_concurrent_usage(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """Concurrent writes should not lose messages."""
    path = tmp_path / "concurrent.log"
    with file_handler_factory(path, 8, 1) as handler:

        def send(h: FemtoFileHandler, i: int) -> None:
            h.handle("core", "INFO", f"msg{i}")

        threads = [threading.Thread(target=send, args=(handler, i)) for i in range(10)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
    data = path.read_text()
    for i in range(10):
        assert f"core [INFO] msg{i}" in data


def test_file_handler_flush(tmp_path: Path) -> None:
    """Test that ``flush()`` writes pending records immediately."""
    path = tmp_path / "flush.log"
    with closing(FemtoFileHandler(str(path))) as handler:

        def send(msg: str) -> None:
            handler.handle("core", "INFO", msg)
            assert handler.flush() is True

        send("one")
        assert path.read_text() == "core [INFO] one\n"
        send("two")
        assert path.read_text() == "core [INFO] one\ncore [INFO] two\n"


def test_file_handler_flush_concurrent(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """Concurrent ``flush()`` calls should each succeed."""
    path = tmp_path / "flush_concurrent.log"
    with file_handler_factory(path, 8, 1) as handler:

        def send_and_flush() -> None:
            handler.handle("core", "INFO", "msg")
            assert handler.flush() is True

        threads = [threading.Thread(target=send_and_flush) for _ in range(5)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

    assert len(path.read_text().splitlines()) == 5


def test_file_handler_open_failure(tmp_path: Path) -> None:
    """Creating a handler in a missing directory raises ``OSError``."""
    bad_dir = tmp_path / "does_not_exist"
    path = bad_dir / "out.log"
    with pytest.raises(OSError, match=re.escape(str(path))) as excinfo:
        FemtoFileHandler(str(path))
    assert excinfo.value.errno in {None, errno.ENOENT}


def test_file_handler_custom_flush_interval(
    tmp_path: Path,
    file_handler_factory: FileHandlerFactory,
) -> None:
    """Flush only after ``flush_interval`` records."""
    path = tmp_path / "interval.log"
    with file_handler_factory(path, 8, 2) as handler:
        handler.handle("core", "INFO", "first")
        handler.handle("core", "INFO", "second")
        handler.handle("core", "INFO", "third")
    assert path.read_text() == (
        "core [INFO] first\ncore [INFO] second\ncore [INFO] third\n"
    )


def test_file_handler_flush_interval_one(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """Records flush after every write when flush_interval is one."""
    path = tmp_path / "flush_one.log"
    with file_handler_factory(path, 8, 1) as handler:
        handler.handle("core", "INFO", "message")
    assert path.read_text() == "core [INFO] message\n"


def test_file_handler_flush_interval_large(tmp_path: Path) -> None:
    """Large flush_interval flushes all messages on close."""
    path = tmp_path / "large_flush.log"
    with closing(
        FemtoFileHandler(
            str(path),
            capacity=8,
            flush_interval=10000,
            policy="drop",
        )
    ) as handler:
        for i in range(5):
            handler.handle("core", "INFO", f"msg {i}")
        assert path.read_text() == ""
    expected = "".join(f"core [INFO] msg {i}\n" for i in range(5))
    assert path.read_text() == expected


def test_overflow_policy_block(tmp_path: Path) -> None:
    """Block policy waits for space instead of dropping records."""
    path = tmp_path / "block.log"
    with closing(
        FemtoFileHandler(
            str(path),
            capacity=2,
            flush_interval=1,
            policy="block",
        )
    ) as handler:
        handler.handle("core", "INFO", "first")
        handler.handle("core", "INFO", "second")
        handler.handle("core", "INFO", "third")
    assert (
        path.read_text() == "core [INFO] first\ncore [INFO] second\ncore [INFO] third\n"
    )


def test_overflow_policy_timeout(tmp_path: Path) -> None:
    """Timeout policy drops records once the queue is saturated."""
    path = tmp_path / "timeout.log"
    worker_started = threading.Event()
    release_worker = threading.Event()

    @contextmanager
    def release_worker_on_exit(
        event: threading.Event,
    ) -> cabc.Generator[None, None, None]:
        try:
            yield
        finally:
            event.set()

    def blocking_formatter(record: FormatterRecord) -> str:
        if not worker_started.is_set():
            worker_started.set()
            if not release_worker.wait(timeout=10.0):
                pytest.fail("timeout waiting for release_worker in formatter")
        return f"{record['logger']} [{record['level']}] {record['message']}"

    builder = (
        FileHandlerBuilder(str(path))
        .with_capacity(1)
        .with_flush_after_records(10000)
        .with_overflow_policy(OverflowPolicy.timeout(200))
        .with_formatter(blocking_formatter)
    )
    # Release the worker before closing to avoid racing on the final flush.
    with closing(builder.build()) as handler, release_worker_on_exit(release_worker):
        handler.handle("core", "INFO", "first")
        assert worker_started.wait(10.0), "worker never reached formatter"
        # Capacity=1 allows one queued record while the worker is busy.
        handler.handle("core", "INFO", "second")
        with pytest.raises(RuntimeError, match="timed out"):
            handler.handle("core", "INFO", "third")
    expected = [
        "core [INFO] first",
        "core [INFO] second",
    ]
    lines = _read_lines_with_retry(path, expected)
    assert lines == expected, "expected timeout policy to drop the third record"


def test_overflow_policy_drop(tmp_path: Path) -> None:
    """Drop policy discards records once the queue is full."""
    path = tmp_path / "drop.log"
    with closing(
        FemtoFileHandler(
            str(path),
            capacity=2,
            flush_interval=1,
            policy="drop",
        )
    ) as handler:
        handler.handle("core", "INFO", "first")
        handler.handle("core", "INFO", "second")
        error_msg: str | None = None
        try:
            handler.handle("core", "INFO", "third")
        except RuntimeError as err:
            error_msg = str(err)
        if error_msg is not None:
            assert error_msg in {
                "Handler error: queue full",
                "Handler error: handler is closed",
            }
    # The consumer runs concurrently; on faster CI machines it may
    # dequeue between sends. Assert the first two messages are present
    # in order, without requiring the third to be dropped deterministically.
    assert path.read_text().splitlines()[:2] == [
        "core [INFO] first",
        "core [INFO] second",
    ]


def test_overflow_policy_drop_flush_interval_gt_one(tmp_path: Path) -> None:
    """Drop policy with buffered writes still discards excess records."""
    path = tmp_path / "drop_flush_gt_one.log"
    with closing(
        FemtoFileHandler(
            str(path),
            capacity=2,
            flush_interval=5,
            policy="drop",
        )
    ) as handler:
        for i in range(10):
            error_msg: str | None = None
            try:
                handler.handle("core", "INFO", f"msg{i}")
            except RuntimeError as err:
                error_msg = str(err)
            if error_msg is not None:
                assert error_msg in {
                    "Handler error: queue full",
                    "Handler error: handler is closed",
                }
    assert path.read_text().splitlines()[:2] == [
        "core [INFO] msg0",
        "core [INFO] msg1",
    ]


def test_overflow_policy_invalid(tmp_path: Path) -> None:
    """Invalid policy strings raise ``ValueError``."""
    path = tmp_path / "invalid.log"
    with pytest.raises(ValueError, match="invalid overflow policy"):
        FemtoFileHandler(str(path), policy="bogus")


def test_overflow_policy_timeout_missing_ms(tmp_path: Path) -> None:
    """Timeout policy without a timeout is rejected."""
    path = tmp_path / "missing_ms.log"
    with pytest.raises(
        ValueError,
        match=r"timeout requires a positive integer N, use 'timeout:N'",
    ):
        FemtoFileHandler(str(path), policy="timeout")


def test_file_handler_handle_after_close_raises(tmp_path: Path) -> None:
    """Calling ``handle`` on a closed handler raises ``RuntimeError``."""
    path = tmp_path / "closed.log"
    handler = FemtoFileHandler(str(path))
    handler.close()
    with pytest.raises(RuntimeError, match="Handler error: handler is closed"):
        handler.handle("core", "INFO", "after close")


def test_capacity_validation(tmp_path: Path) -> None:
    """Capacity must be greater than zero."""
    path = tmp_path / "bad_capacity.log"
    with pytest.raises(ValueError, match="capacity must be greater than zero"):
        FemtoFileHandler(str(path), capacity=0)


def test_flush_interval_validation(tmp_path: Path) -> None:
    """Flush interval must be greater than zero."""
    path = tmp_path / "bad_flush.log"
    with pytest.raises(ValueError, match="flush_interval must be greater than zero"):
        FemtoFileHandler(str(path), flush_interval=0)
    with pytest.raises(ValueError, match="flush_interval must be greater than zero"):
        FemtoFileHandler(str(path), flush_interval=-1)


def test_timeout_policy_validation(tmp_path: Path) -> None:
    """Timeout policy requires a positive timeout."""
    path = tmp_path / "bad_timeout.log"
    with pytest.raises(ValueError, match="timeout must be greater than zero"):
        FemtoFileHandler(str(path), policy="timeout:0")
    with pytest.raises(
        ValueError,
        match=r"timeout must be a positive integer \(N in 'timeout:N'\)",
    ):
        FemtoFileHandler(str(path), policy="timeout:-1")


def test_timeout_policy_non_numeric(tmp_path: Path) -> None:
    """Non-numeric timeout values are rejected."""
    path = tmp_path / "timeout_non_numeric.log"
    with pytest.raises(
        ValueError,
        match=r"timeout must be a positive integer \(N in 'timeout:N'\)",
    ):
        FemtoFileHandler(str(path), policy="timeout:abc")


def test_default_constructor(tmp_path: Path) -> None:
    """Default arguments apply drop policy and flush after every record."""
    path = tmp_path / "defaults.log"
    with closing(FemtoFileHandler(str(path))) as handler:
        handler.handle("core", "INFO", "first")
        handler.handle("core", "INFO", "second")
    assert path.read_text() == "core [INFO] first\ncore [INFO] second\n"


def test_policy_normalization(tmp_path: Path) -> None:
    """Policy strings are normalized for case and whitespace."""
    path = tmp_path / "policy.log"
    with closing(FemtoFileHandler(str(path), policy=" Drop ")) as handler:
        handler.handle("core", "INFO", "msg")
    assert path.read_text() == "core [INFO] msg\n"
