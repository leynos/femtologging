"""Tests for the FemtoFileHandler."""

from __future__ import annotations

import collections.abc as cabc
from pathlib import Path
import threading
import typing

from femtologging import FemtoFileHandler
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
