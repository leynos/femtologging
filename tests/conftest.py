from __future__ import annotations

import gc
from contextlib import contextmanager
from pathlib import Path
from typing import Callable, ContextManager, Generator

from femtologging import FemtoFileHandler, OverflowPolicy, PyHandlerConfig
import pytest

FileHandlerFactory = Callable[[Path, int, int], ContextManager[FemtoFileHandler]]


@pytest.fixture()
def file_handler_factory() -> FileHandlerFactory:
    """Return a context manager creating a ``FemtoFileHandler``.

    The factory yields a handler that flushes every ``flush_interval`` records.
    The handler is automatically closed when the ``with`` block exits to ensure
    the worker thread shuts down.
    """

    @contextmanager
    def factory(
        path: Path, capacity: int, flush_interval: int
    ) -> Generator[FemtoFileHandler, None, None]:
        cfg = PyHandlerConfig(
            capacity,
            flush_interval,
            OverflowPolicy.DROP.value,
            timeout_ms=None,
        )
        handler = FemtoFileHandler.with_capacity_flush_policy(str(path), cfg)
        try:
            yield handler
        finally:
            if hasattr(handler, "close"):
                handler.close()
            del handler
            gc.collect()

    return factory
