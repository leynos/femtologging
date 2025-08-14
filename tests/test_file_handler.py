"""Tests for the FemtoFileHandler."""

from __future__ import annotations

import collections.abc as cabc
from pathlib import Path
import threading
import typing
from contextlib import closing

from femtologging import FemtoFileHandler, FemtoFileHandlerConfig, OverflowPolicy
import pytest

FileHandlerFactory = cabc.Callable[
    [Path, int, int], typing.ContextManager[FemtoFileHandler]
]


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
    with pytest.raises(OSError):
        FemtoFileHandler(str(path))


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
    cfg = FemtoFileHandlerConfig(
        capacity=8,
        flush_interval=10000,
        policy=OverflowPolicy.DROP.value,
    )
    with closing(FemtoFileHandler(str(path), cfg)) as handler:
        for i in range(5):
            handler.handle("core", "INFO", f"msg {i}")
        assert path.read_text() == ""
    expected = "".join(f"core [INFO] msg {i}\n" for i in range(5))
    assert path.read_text() == expected


def test_overflow_policy_block(tmp_path: Path) -> None:
    """Block policy waits for space before dropping records."""
    path = tmp_path / "block.log"
    cfg = FemtoFileHandlerConfig(capacity=2, flush_interval=1, policy="block")
    with closing(FemtoFileHandler(str(path), cfg)) as handler:
        handler.handle("core", "INFO", "first")
        handler.handle("core", "INFO", "second")
        handler.handle("core", "INFO", "third")
    assert (
        path.read_text() == "core [INFO] first\ncore [INFO] second\ncore [INFO] third\n"
    )


def test_overflow_policy_timeout(tmp_path: Path) -> None:
    """Timeout policy honours the timeout."""
    path = tmp_path / "timeout.log"
    cfg = FemtoFileHandlerConfig(
        capacity=1,
        flush_interval=1,
        policy="timeout",
        timeout_ms=500,
    )
    with closing(FemtoFileHandler(str(path), cfg)) as handler:
        handler.handle("core", "INFO", "first")
    assert path.read_text() == "core [INFO] first\n"


def test_overflow_policy_drop(tmp_path: Path) -> None:
    """Drop policy discards records once the queue is full."""
    path = tmp_path / "drop.log"
    cfg = FemtoFileHandlerConfig(capacity=2, flush_interval=1, policy="drop")
    with closing(FemtoFileHandler(str(path), cfg)) as handler:
        handler.handle("core", "INFO", "first")
        handler.handle("core", "INFO", "second")
        handler.handle("core", "INFO", "third")
    assert path.read_text() == "core [INFO] first\ncore [INFO] second\n"


def test_overflow_policy_drop_flush_interval_gt_one(tmp_path: Path) -> None:
    """Drop policy with buffered writes still discards excess records."""
    path = tmp_path / "drop_flush_gt_one.log"
    cfg = FemtoFileHandlerConfig(capacity=2, flush_interval=5, policy="drop")
    with closing(FemtoFileHandler(str(path), cfg)) as handler:
        handler.handle("core", "INFO", "first")
        handler.handle("core", "INFO", "second")
        handler.handle("core", "INFO", "third")
        handler.handle("core", "INFO", "fourth")
    assert path.read_text() == "core [INFO] first\ncore [INFO] second\n"


def test_overflow_policy_invalid(tmp_path: Path) -> None:
    """Invalid policy strings raise ``ValueError``."""
    path = tmp_path / "invalid.log"
    with pytest.raises(ValueError, match="invalid overflow policy"):
        FemtoFileHandler(str(path), FemtoFileHandlerConfig(policy="bogus"))


def test_overflow_policy_timeout_missing_ms(tmp_path: Path) -> None:
    """Timeout policy without ``timeout_ms`` is rejected."""
    path = tmp_path / "missing_ms.log"
    with pytest.raises(ValueError, match="timeout_ms required"):
        FemtoFileHandler(str(path), FemtoFileHandlerConfig(policy="timeout"))


def test_capacity_validation(tmp_path: Path) -> None:
    """Capacity must be greater than zero."""
    path = tmp_path / "bad_capacity.log"
    with pytest.raises(ValueError, match="capacity must be greater than zero"):
        FemtoFileHandler(str(path), FemtoFileHandlerConfig(capacity=0))


def test_flush_interval_validation(tmp_path: Path) -> None:
    """Flush interval must be greater than zero."""
    path = tmp_path / "bad_flush.log"
    with pytest.raises(ValueError, match="flush_interval must be greater than zero"):
        FemtoFileHandler(str(path), FemtoFileHandlerConfig(flush_interval=0))
    with pytest.raises(ValueError, match="flush_interval must be greater than zero"):
        FemtoFileHandler(str(path), FemtoFileHandlerConfig(flush_interval=-1))


def test_timeout_ms_validation(tmp_path: Path) -> None:
    """Timeout policy requires positive ``timeout_ms``."""
    path = tmp_path / "bad_timeout.log"
    with pytest.raises(ValueError, match="timeout_ms must be greater than zero"):
        FemtoFileHandler(
            str(path),
            FemtoFileHandlerConfig(policy="timeout", timeout_ms=0),
        )
    with pytest.raises(ValueError, match="timeout_ms must be greater than zero"):
        FemtoFileHandler(
            str(path),
            FemtoFileHandlerConfig(policy="timeout", timeout_ms=-1),
        )


def test_default_constructor(tmp_path: Path) -> None:
    """Default arguments apply drop policy and flush after every record."""
    path = tmp_path / "defaults.log"
    with closing(FemtoFileHandler(str(path))) as handler:
        handler.handle("core", "INFO", "first")
        handler.handle("core", "INFO", "second")
    assert path.read_text() == "core [INFO] first\ncore [INFO] second\n"


def test_policy_normalisation(tmp_path: Path) -> None:
    """Policy strings are normalised for case and whitespace."""
    path = tmp_path / "policy.log"
    cfg = FemtoFileHandlerConfig(policy=" Drop ")
    with closing(FemtoFileHandler(str(path), cfg)) as handler:
        handler.handle("core", "INFO", "msg")
    assert path.read_text() == "core [INFO] msg\n"
