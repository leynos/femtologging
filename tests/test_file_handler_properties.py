"""Property-based checks for ``FemtoFileHandler`` argument validation.

The example-based cases in ``tests/test_file_handler.py`` pin the exact error
messages; these properties assert the same relation holds across the whole
non-positive integer domain rather than at a couple of sampled points.

The ``importorskip`` guard below keeps the module collectible in environments
that deliberately install without Hypothesis; every supported interpreter,
including CPython 3.15, does provide it.
"""

from __future__ import annotations

import pytest

from femtologging import FemtoFileHandler

hypothesis = pytest.importorskip("hypothesis")
strategies = pytest.importorskip("hypothesis.strategies")

_NON_POSITIVE = strategies.integers(min_value=-(2**31), max_value=0)


@pytest.fixture(scope="module")
def log_path(tmp_path_factory: pytest.TempPathFactory) -> str:
    """Return a writable log path shared by every generated example.

    A module-scoped path keeps Hypothesis clear of function-scoped fixtures;
    no handler is ever constructed successfully, so nothing is written to it.

    Returns
    -------
    str
        Filesystem path in a temporary directory owned by this module.
    """
    return str(tmp_path_factory.mktemp("handler-properties") / "out.log")


@hypothesis.given(capacity=_NON_POSITIVE)
def test_non_positive_capacity_is_always_rejected(log_path: str, capacity: int) -> None:
    """No non-positive capacity may yield a usable handler."""
    with pytest.raises(ValueError, match="capacity must be greater than zero"):
        FemtoFileHandler(log_path, capacity=capacity)


@hypothesis.given(flush_interval=_NON_POSITIVE)
def test_non_positive_flush_interval_is_always_rejected(
    log_path: str, flush_interval: int
) -> None:
    """No non-positive flush interval may yield a usable handler."""
    with pytest.raises(ValueError, match="flush_interval must be greater than zero"):
        FemtoFileHandler(log_path, flush_interval=flush_interval)
