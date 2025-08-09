from __future__ import annotations

from contextlib import contextmanager
from pathlib import Path
from typing import Iterator
import gc
import sys

# Add project root to sys.path so femtologging can be imported without hacks.
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from femtologging import FemtoFileHandler, OverflowPolicy, PyHandlerConfig
import pytest  # pyright: ignore[reportMissingImports]


@pytest.fixture()
def file_handler_factory():
    """Return a context manager creating a ``FemtoFileHandler``.

    The factory yields a handler that flushes every ``flush_interval`` records.
    The handler is automatically destroyed and garbage collected when the
    ``with`` block exits to ensure the worker thread shuts down.
    """

    @contextmanager
    def factory(
        path: Path, capacity: int, flush_interval: int
    ) -> Iterator[FemtoFileHandler]:
        cfg = PyHandlerConfig(capacity, flush_interval, OverflowPolicy.DROP.value, None)
        handler = FemtoFileHandler.with_capacity_flush_policy(str(path), cfg)
        try:
            yield handler
        finally:
            del handler
            gc.collect()

    return factory
