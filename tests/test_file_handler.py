"""Tests for the FemtoFileHandler."""

from __future__ import annotations

import collections.abc as cabc
from pathlib import Path
import threading
import typing

from femtologging import FemtoFileHandler, OverflowPolicy, PyHandlerConfig
import pytest  # pyright: ignore[reportMissingImports]

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
    handler = FemtoFileHandler(str(path))

    def send(msg: str) -> None:
        handler.handle("core", "INFO", msg)
        assert handler.flush() is True

    send("one")
    assert path.read_text() == "core [INFO] one\n"
    send("two")
    assert path.read_text() == "core [INFO] one\ncore [INFO] two\n"
    handler.close()


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


def test_file_handler_flush_interval_zero(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """Periodic flushing is disabled when flush_interval is zero."""
    path = tmp_path / "flush_zero.log"
    with file_handler_factory(path, 8, 0) as handler:
        handler.handle("core", "INFO", "message")
    assert path.read_text() == "core [INFO] message\n"


def test_file_handler_flush_interval_one(
    tmp_path: Path, file_handler_factory: FileHandlerFactory
) -> None:
    """Records flush after every write when flush_interval is one."""
    path = tmp_path / "flush_one.log"
    with file_handler_factory(path, 8, 1) as handler:
        handler.handle("core", "INFO", "message")
    assert path.read_text() == "core [INFO] message\n"


def test_blocking_policy_basic(tmp_path: Path) -> None:
    """Verify flushing and writing when using the blocking policy."""
    path = tmp_path / "block.log"
    handler = FemtoFileHandler.with_capacity_flush_blocking(str(path), 1, 1)
    handler.handle("core", "INFO", "first")
    handler.close()
    assert path.read_text() == "core [INFO] first\n"


def test_blocking_policy_over_capacity(tmp_path: Path) -> None:
    """Verify blocking behaviour when capacity is exceeded."""
    path = tmp_path / "block_over.log"
    handler = FemtoFileHandler.with_capacity_flush_blocking(str(path), 2, 1)
    handler.handle("core", "INFO", "first")
    handler.handle("core", "INFO", "second")
    handler.handle("core", "INFO", "third")
    handler.close()
    assert (
        path.read_text() == "core [INFO] first\ncore [INFO] second\ncore [INFO] third\n"
    )


def test_timeout_policy_basic(tmp_path: Path) -> None:
    """Test basic functionality of timeout policy in FemtoFileHandler."""
    path = tmp_path / "timeout.log"
    handler = FemtoFileHandler.with_capacity_flush_timeout(
        str(path), 1, 1, timeout_ms=500
    )
    handler.handle("core", "INFO", "first")
    handler.close()
    assert path.read_text() == "core [INFO] first\n"


def test_timeout_policy_over_capacity(tmp_path: Path) -> None:
    """Ensure timeout policy flushes when over capacity."""
    path = tmp_path / "timeout_over.log"
    handler = FemtoFileHandler.with_capacity_flush_timeout(
        str(path), 2, 1, timeout_ms=1000
    )
    handler.handle("core", "INFO", "first")
    handler.handle("core", "INFO", "second")
    handler.handle("core", "INFO", "third")
    handler.close()
    assert (
        path.read_text() == "core [INFO] first\ncore [INFO] second\ncore [INFO] third\n"
    )


def test_overflow_policy_builder_block(tmp_path: Path) -> None:
    """Overflow policy can be specified explicitly using strings."""
    path = tmp_path / "block_enum.log"
    cfg = PyHandlerConfig(2, 1, OverflowPolicy.BLOCK.value, None)
    handler = FemtoFileHandler.with_capacity_flush_policy(str(path), cfg)
    handler.handle("core", "INFO", "first")
    handler.handle("core", "INFO", "second")
    handler.handle("core", "INFO", "third")
    handler.close()
    assert (
        path.read_text() == "core [INFO] first\ncore [INFO] second\ncore [INFO] third\n"
    )


def test_overflow_policy_builder_timeout(tmp_path: Path) -> None:
    """Timeout policy via builder honours the timeout."""
    path = tmp_path / "builder_timeout.log"
    cfg = PyHandlerConfig(1, 1, OverflowPolicy.TIMEOUT.value, timeout_ms=500)
    handler = FemtoFileHandler.with_capacity_flush_policy(str(path), cfg)
    handler.handle("core", "INFO", "first")
    handler.close()
    assert path.read_text() == "core [INFO] first\n"


def test_overflow_policy_builder_drop(tmp_path: Path) -> None:
    """Drop policy discards records once the queue is full."""
    path = tmp_path / "drop_enum.log"
    cfg = PyHandlerConfig(2, 1, OverflowPolicy.DROP.value, None)
    handler = FemtoFileHandler.with_capacity_flush_policy(str(path), cfg)
    handler.handle("core", "INFO", "first")
    handler.handle("core", "INFO", "second")
    handler.handle("core", "INFO", "third")  # dropped
    handler.close()
    assert path.read_text() == "core [INFO] first\ncore [INFO] second\n"


def test_overflow_policy_builder_invalid(tmp_path: Path) -> None:
    """Invalid policy strings raise ``ValueError``."""
    path = tmp_path / "invalid.log"
    with pytest.raises(ValueError):
        FemtoFileHandler.with_capacity_flush_policy(
            str(path), PyHandlerConfig(1, 1, "bogus", None)
        )


def test_overflow_policy_builder_timeout_missing_ms(tmp_path: Path) -> None:
    """Timeout policy without ``timeout_ms`` is rejected."""
    path = tmp_path / "missing_ms.log"
    with pytest.raises(ValueError):
        FemtoFileHandler.with_capacity_flush_policy(
            str(path), PyHandlerConfig(1, 1, OverflowPolicy.TIMEOUT.value, None)
        )


def test_py_handler_config_mutation(tmp_path: Path) -> None:
    """Mutating a ``PyHandlerConfig`` before use affects the handler."""
    cfg = PyHandlerConfig(1, 10, OverflowPolicy.BLOCK.value, None)
    cfg.capacity = 2
    cfg.flush_interval = 1
    cfg.policy = OverflowPolicy.DROP.value
    cfg.timeout_ms = None
    path = tmp_path / "mutate.log"
    handler = FemtoFileHandler.with_capacity_flush_policy(str(path), cfg)
    handler.handle("core", "INFO", "first")
    handler.handle("core", "INFO", "second")
    handler.handle("core", "INFO", "third")  # dropped due to capacity 2
    handler.close()
    assert path.read_text() == "core [INFO] first\ncore [INFO] second\n"


def test_py_handler_config_invalid_capacity() -> None:
    """Capacity must be greater than zero."""
    with pytest.raises(ValueError) as exc_info:
        PyHandlerConfig(0, 1, OverflowPolicy.DROP.value, None)
    assert "capacity must be greater than zero" in str(exc_info.value)


def test_py_handler_config_invalid_flush_interval() -> None:
    """Flush interval must be greater than zero."""
    with pytest.raises(ValueError) as exc_info:
        PyHandlerConfig(1, 0, OverflowPolicy.DROP.value, None)
    assert "flush_interval must be greater than zero" in str(exc_info.value)


def test_py_handler_config_set_capacity_invalid() -> None:
    """Setting capacity to zero raises ``ValueError``."""
    cfg = PyHandlerConfig(1, 1, OverflowPolicy.DROP.value, None)
    with pytest.raises(ValueError) as exc_info:
        cfg.capacity = 0
    assert "capacity must be greater than zero" in str(exc_info.value)


def test_py_handler_config_set_flush_interval_invalid() -> None:
    """Setting flush_interval to zero raises ``ValueError``."""
    cfg = PyHandlerConfig(1, 1, OverflowPolicy.DROP.value, None)
    with pytest.raises(ValueError) as exc_info:
        cfg.flush_interval = 0
    assert "flush_interval must be greater than zero" in str(exc_info.value)


def test_py_handler_config_set_policy_invalid() -> None:
    """Setting an invalid policy raises ``ValueError``."""
    cfg = PyHandlerConfig(1, 1, OverflowPolicy.DROP.value, None)
    with pytest.raises(ValueError):
        cfg.policy = "bogus"
