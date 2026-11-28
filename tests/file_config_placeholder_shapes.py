"""Scan shapes and expected matches for the bounded-work placeholder tests.

Each builder returns one input whose repeated unit stays well below the handler
parameter limit at the largest tested size. The expected matches are declared
here rather than derived from the regex oracle, which is itself quadratic on
the malformed shapes and would reintroduce the cost under test.
"""

from __future__ import annotations

import typing as typ

import pytest

if typ.TYPE_CHECKING:
    import collections.abc as cabc

type Match = tuple[int, int, str]
type ExpectedMatches = cabc.Callable[[int], list[Match]]


def _unterminated_openers(size: int) -> str:
    """Return *size* ``%(`` openers with no closer at all."""
    return "%(" * size


def _failed_closers(size: int) -> str:
    """Return *size* ``%(a)`` candidates whose closer is not followed by ``s``."""
    return "%(a)" * size


def _nested_openers(size: int) -> str:
    """Return *size* nested openers resolved by one trailing ``)s``."""
    return "%(x" * size + ")s"


def _adjacent_valid(size: int) -> str:
    """Return *size* adjacent well-formed placeholders named ``a``."""
    return "%(a)s" * size


def _distant_closer(size: int) -> str:
    """Return *size* openers sharing one closer that is not followed by ``s``.

    Every candidate reaches the same distant closer and is rejected there, so
    the shape exercises the resume point directly. Resuming at the opener
    rather than just past the closer re-searches that whole span for each
    opener, which is the quadratic path the scanner documents it avoids.

    Returns
    -------
    str
        *size* openers followed by one rejected closer.

    """
    return "%(" * size + ")x"


def _no_matches(_size: int) -> list[Match]:
    """Return the matches a shape holding no valid placeholder yields."""
    return []


def _one_spanning_match(size: int) -> list[Match]:
    """Return the single match consuming the whole nested-opener input."""
    name = "x" + "%(x" * (size - 1)
    return [(0, len(_nested_openers(size)), name)]


def _adjacent_matches(size: int) -> list[Match]:
    """Return one five-character ``a`` match per adjacent placeholder."""
    stride = len("%(a)s")
    return [(stride * index, stride * index + stride, "a") for index in range(size)]


SCAN_BUILDERS = [
    pytest.param(_unterminated_openers, _no_matches, id="unterminated-openers"),
    pytest.param(_failed_closers, _no_matches, id="failed-closers"),
    pytest.param(_nested_openers, _one_spanning_match, id="nested-openers"),
    pytest.param(_adjacent_valid, _adjacent_matches, id="adjacent-valid"),
    pytest.param(_distant_closer, _no_matches, id="distant-closer"),
]
