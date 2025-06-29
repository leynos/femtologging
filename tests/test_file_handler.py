# pyright: reportMissingImports=false
"""Tests for the FemtoFileHandler."""

from __future__ import annotations

from pathlib import Path
import threading

from femtologging import FemtoFileHandler
import pytest  # pyright: ignore[reportMissingImports]


def test_file_handler_writes_to_file(tmp_path: Path) -> None:
    path = tmp_path / "out.log"
    handler = FemtoFileHandler(str(path))
    handler.handle("core", "INFO", "hello")
    assert handler.flush() is True
    handler.close()
    assert path.read_text() == "core [INFO] hello\n"


def test_file_handler_multiple_records(tmp_path: Path) -> None:
    path = tmp_path / "multi.log"
    handler = FemtoFileHandler.with_capacity(str(path), 8)
    handler.handle("core", "INFO", "first")
    handler.handle("core", "WARN", "second")
    handler.handle("core", "ERROR", "third")
    handler.close()
    assert (
        path.read_text()
        == "core [INFO] first\ncore [WARN] second\ncore [ERROR] third\n"
    )


def test_file_handler_concurrent_usage(tmp_path: Path) -> None:
    path = tmp_path / "concurrent.log"
    handler = FemtoFileHandler(str(path))

    def send(h: FemtoFileHandler, i: int) -> None:
        h.handle("core", "INFO", f"msg{i}")

    threads = [threading.Thread(target=send, args=(handler, i)) for i in range(10)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    handler.close()
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


def test_file_handler_open_failure(tmp_path: Path) -> None:
    bad_dir = tmp_path / "does_not_exist"
    path = bad_dir / "out.log"
    with pytest.raises(OSError):
        FemtoFileHandler(str(path))


def test_file_handler_custom_flush_interval(tmp_path: Path) -> None:
    path = tmp_path / "interval.log"
    handler = FemtoFileHandler.with_capacity_flush(str(path), 8, 2)
    handler.handle("core", "INFO", "first")
    handler.handle("core", "INFO", "second")
    handler.handle("core", "INFO", "third")
    del handler
    gc.collect()
    assert path.read_text() == (
        "core [INFO] first\ncore [INFO] second\ncore [INFO] third\n"
    )


def test_file_handler_flush_interval_zero(tmp_path: Path) -> None:
    """Periodic flushing is disabled when flush_interval is zero."""
    path = tmp_path / "flush_zero.log"
    handler = FemtoFileHandler.with_capacity_flush(str(path), 8, 0)
    handler.handle("core", "INFO", "message")
    del handler
    gc.collect()
    assert path.read_text() == "core [INFO] message\n"


def test_file_handler_flush_interval_one(tmp_path: Path) -> None:
    """Records flush after every write when flush_interval is one."""
    path = tmp_path / "flush_one.log"
    handler = FemtoFileHandler.with_capacity_flush(str(path), 8, 1)
    handler.handle("core", "INFO", "message")
    del handler
    gc.collect()
    assert path.read_text() == "core [INFO] message\n"
