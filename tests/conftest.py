"""Shared pytest fixtures for femtologging tests."""

from __future__ import annotations

import collections.abc as cabc
import gc
import warnings
from contextlib import AbstractContextManager, contextmanager
from pathlib import Path

import pytest

import femtologging
from femtologging import FemtoFileHandler

warnings.filterwarnings(
    "ignore",
    message="'maxsplit' is passed as positional argument",
    category=DeprecationWarning,
    module=r"gherkin\.gherkin_line",
)
# The warning originates in the vendored Gherkin parser, so filter it out until
# the dependency releases a fix rather than letting our test suite go noisy.

type FileHandlerFactory = cabc.Callable[
    [Path, int, int], AbstractContextManager[FemtoFileHandler]
]


@pytest.fixture
def file_handler_factory() -> FileHandlerFactory:
    """Return a context manager creating a ``FemtoFileHandler``.

    The factory yields a handler that flushes every ``flush_interval`` records.
    The handler is automatically closed when the ``with`` block exits to ensure
    the worker thread shuts down.

    Returns
    -------
    FileHandlerFactory
        Callable taking ``(path, capacity, flush_interval)`` and returning a
        context manager that yields the configured handler.
    """

    @contextmanager
    def factory(
        path: Path, capacity: int, flush_interval: int
    ) -> cabc.Generator[FemtoFileHandler, None, None]:
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


@pytest.fixture
def active_exception() -> cabc.Callable[[str], AbstractContextManager[None]]:
    """Return a context manager that establishes an active exception.

    The returned callable accepts a message and yields inside a
    ``try/except`` block so that ``sys.exc_info()`` is populated for
    the duration of the ``with`` block.

    .. note::

        Because ``@contextmanager`` creates a generator frame,
        ``sys.exc_info()`` is scoped to that frame and is **not**
        visible to the ``with``-body in the calling test.  Tests that
        need the exception visible to ``sys.exc_info()`` in their own
        frame must still use an inline ``try/except``.  This fixture
        is useful for tests where an active handler context is
        required but the captured traceback content is not asserted.

    Returns
    -------
    cabc.Callable[[str], AbstractContextManager[None]]
        Callable taking the exception message and returning a context manager
        whose body runs inside an ``except ValueError`` block.

    Examples
    --------
    >>> def test_example(active_exception):
    ...     with active_exception("boom"):
    ...         output = logger.exception("caught")

    """

    @contextmanager
    def _raise(message: str = "test error") -> cabc.Generator[None, None, None]:
        try:
            # ruff: ignore[raise-within-try] the raise is the fixture's whole
            # purpose: it populates sys.exc_info() for the generator frame.
            raise ValueError(message)
        except ValueError:
            yield

    return _raise


@pytest.fixture(autouse=True)
def _clean_logging_manager() -> cabc.Generator[None, None, None]:
    """Reset global logger manager before and after each test."""
    femtologging.reset_manager()
    try:
        yield
    finally:
        femtologging.reset_manager()
