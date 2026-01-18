"""Shared types and helpers for frame filter BDD steps.

This module provides the FilterFixture TypedDict and helper functions used
across Given, When, and Then step definitions.

Types exported
--------------
- FilterFixture: TypedDict for shared test state storage

Helper functions
----------------
- _parse_filenames: Parse comma-separated quoted filenames
- _get_frames: Extract frames from filtered payload or its cause

Fixtures
--------
- filter_fixture: pytest fixture providing FilterFixture storage
"""

from __future__ import annotations

import typing as typ

import pytest

if typ.TYPE_CHECKING:
    from tests.frame_filter.conftest import (
        ExceptionPayload,
        FrameDict,
        StackPayload,
    )


class FilterFixture(typ.TypedDict, total=False):
    """State storage for filter fixture."""

    payload: StackPayload | ExceptionPayload
    filtered: StackPayload | ExceptionPayload
    patterns: list[str]


def _parse_filenames(filenames_str: str) -> list[str]:
    """Parse a comma-separated list of quoted filenames.

    Parameters
    ----------
    filenames_str : str
        Comma-separated quoted filenames (e.g., '"a.py", "b.py"').

    Returns
    -------
    list[str]
        List of unquoted filename strings. Empty input or trailing commas
        produce no bogus empty-string entries.

    """
    # Input: '"a.py", "b.py", "c.py"'
    # Output: ['a.py', 'b.py', 'c.py']
    return [name for f in filenames_str.split(",") if (name := f.strip().strip('"'))]


def _get_frames(
    filter_fixture: FilterFixture,
    *,
    from_cause: bool = False,
) -> list[FrameDict]:
    """Extract frames from filtered payload or its cause.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the filtered result.
    from_cause : bool, optional
        If True, extract frames from the cause exception instead.

    Returns
    -------
    list[FrameDict]
        List of frame dicts from the appropriate location.

    """
    filtered = filter_fixture["filtered"]
    if from_cause:
        cause = typ.cast("ExceptionPayload", filtered).get(
            "cause", typ.cast("ExceptionPayload", {})
        )
        return cause.get("frames", [])
    return filtered.get("frames", [])


@pytest.fixture
def filter_fixture() -> FilterFixture:
    """Storage for filter state."""
    return typ.cast("FilterFixture", {})
