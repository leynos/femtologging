from __future__ import annotations

import gc
from contextlib import contextmanager
from pathlib import Path
from typing import Callable, ContextManager, Generator

import femtologging
from femtologging import FemtoFileHandler
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
        handler = FemtoFileHandler(
            str(path),
            capacity=capacity,
            flush_interval=flush_interval,
            policy="drop",
        )
        try:
            yield handler
        finally:
            # Ensure the worker thread shuts down deterministically.
            # Close explicitly, then force finalization in case any ref-cycles
            # or delayed drops remain that could otherwise leak threads in CI.
            handler.close()
            del handler
            gc.collect()

    return factory


@pytest.fixture(autouse=True)
def _reset_manager() -> Generator[None, None, None]:
    """Reset global logger manager before and after each test."""
    femtologging.reset_manager()
    try:
        yield
    finally:
        femtologging.reset_manager()
