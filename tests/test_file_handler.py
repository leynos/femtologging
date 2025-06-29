from __future__ import annotations

import gc
from pathlib import Path
import threading

from femtologging import FemtoFileHandler


def test_file_handler_writes_to_file(tmp_path: Path) -> None:
    path = tmp_path / "out.log"
    handler = FemtoFileHandler(str(path))
    handler.handle("core", "INFO", "hello")
    del handler
    gc.collect()
    assert path.read_text() == "core [INFO] hello\n"


def test_file_handler_multiple_records(tmp_path: Path) -> None:
    path = tmp_path / "multi.log"
    handler = FemtoFileHandler.with_capacity(str(path), 8)
    handler.handle("core", "INFO", "first")
    handler.handle("core", "WARN", "second")
    handler.handle("core", "ERROR", "third")
    del handler
    gc.collect()
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
    del handler
    gc.collect()
    data = path.read_text()
    for i in range(10):
        assert f"core [INFO] msg{i}" in data
