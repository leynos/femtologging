"""BDD step definitions for frame filtering scenarios.

This module provides pytest-bdd step functions that implement the Gherkin
scenarios defined in ``tests/features/frame_filter.feature``. The steps
exercise the ``filter_frames`` and ``get_logging_infrastructure_patterns``
functions from the femtologging package.

Scenarios covered
-----------------
- Filtering frames by filename pattern (exclude_filenames parameter)
- Filtering logging infrastructure frames (exclude_logging parameter)
- Limiting stack depth (max_depth parameter)
- Combined filtering with multiple parameters
- Recursive filtering on exception cause chains
- Retrieving logging infrastructure patterns

Step definitions
----------------
Given steps (payload creation):
    - ``create_stack_payload``: Creates a stack_info payload from filenames
    - ``create_exception_payload``: Creates an exception payload from filenames
    - ``add_cause_to_exception``: Adds a cause exception to an existing payload

When steps (filter operations):
    - ``filter_exclude_logging``: Filters with exclude_logging=True
    - ``filter_exclude_filenames``: Filters with an exclude_filenames pattern
    - ``filter_max_depth``: Filters with a max_depth limit
    - ``filter_combined``: Filters with multiple parameters combined
    - ``get_patterns``: Retrieves logging infrastructure patterns

Then steps (assertions):
    - ``check_frame_count``: Asserts filtered frame count
    - ``check_frame_filename``: Asserts single frame filename
    - ``check_frames_exclude_pattern``: Asserts no frame contains pattern
    - ``check_frames_order``: Asserts two frames in expected order
    - ``check_cause_frame_count``: Asserts cause exception frame count
    - ``check_cause_frame_filename``: Asserts single cause frame filename
    - ``check_patterns_contain``: Asserts pattern list contains value

Fixtures
--------
- ``filter_fixture``: A TypedDict storing payload, filtered result, and patterns

Usage
-----
Run the frame filter scenarios with pytest::

    pytest tests/features/frame_filter.feature -v

Or run all BDD tests::

    pytest tests/ -k "feature" -v
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import filter_frames, get_logging_infrastructure_patterns
from tests.frame_filter.conftest import (
    ExceptionPayload,
    FrameDict,
    StackPayload,
    make_exception_payload,
    make_stack_payload,
)


class FilterFixture(typ.TypedDict, total=False):
    """State storage for filter fixture."""

    payload: StackPayload | ExceptionPayload
    filtered: StackPayload | ExceptionPayload
    patterns: list[str]


FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "frame_filter.feature"))


def _parse_filenames(filenames_str: str) -> list[str]:
    """Parse a comma-separated list of quoted filenames."""
    # Input: '"a.py", "b.py", "c.py"'
    # Output: ['a.py', 'b.py', 'c.py']
    return [f.strip().strip('"') for f in filenames_str.split(",")]


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


@given(parsers.parse("a stack_info payload with frames from {filenames}"))
def create_stack_payload(filter_fixture: FilterFixture, filenames: str) -> None:
    """Create a stack_info payload from comma-separated filenames.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage.
    filenames : str
        Comma-separated quoted filenames (e.g., '"a.py", "b.py"').

    Returns
    -------
    None
        Stores payload in filter_fixture["payload"].

    """
    parsed = _parse_filenames(filenames)
    filter_fixture["payload"] = make_stack_payload(parsed)


@given(parsers.parse("an exception payload with frames from {filenames}"))
def create_exception_payload(filter_fixture: FilterFixture, filenames: str) -> None:
    """Create an exception payload from comma-separated filenames.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage.
    filenames : str
        Comma-separated quoted filenames (e.g., '"a.py", "b.py"').

    Returns
    -------
    None
        Stores payload in filter_fixture["payload"].

    """
    parsed = _parse_filenames(filenames)
    filter_fixture["payload"] = make_exception_payload(parsed, type_name="TestError")


@given(parsers.parse("the exception has a cause with frames from {filenames}"))
def add_cause_to_exception(filter_fixture: FilterFixture, filenames: str) -> None:
    """Add a cause exception to the existing exception payload.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing an exception payload.
    filenames : str
        Comma-separated quoted filenames for the cause frames.

    Returns
    -------
    None
        Adds cause to filter_fixture["payload"]["cause"].

    """
    parsed = _parse_filenames(filenames)
    cause = make_exception_payload(
        parsed, type_name="CauseError", message="cause error"
    )
    typ.cast("ExceptionPayload", filter_fixture["payload"])["cause"] = cause


@when("I filter with exclude_logging=True")
def filter_exclude_logging(filter_fixture: FilterFixture) -> None:
    """Filter the payload excluding logging infrastructure frames.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the payload to filter.

    Returns
    -------
    None
        Stores filtered result in filter_fixture["filtered"].

    """
    filter_fixture["filtered"] = typ.cast(
        "StackPayload | ExceptionPayload",
        filter_frames(filter_fixture["payload"], exclude_logging=True),
    )


@when(parsers.parse('I filter with exclude_filenames=["{pattern}"]'))
def filter_exclude_filenames(filter_fixture: FilterFixture, pattern: str) -> None:
    """Filter the payload excluding frames matching the filename pattern.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the payload to filter.
    pattern : str
        Substring pattern to match against frame filenames.

    Returns
    -------
    None
        Stores filtered result in filter_fixture["filtered"].

    """
    filter_fixture["filtered"] = typ.cast(
        "StackPayload | ExceptionPayload",
        filter_frames(filter_fixture["payload"], exclude_filenames=[pattern]),
    )


@when(parsers.parse("I filter with max_depth={n:d}"))
def filter_max_depth(filter_fixture: FilterFixture, n: int) -> None:
    """Filter the payload limiting to the deepest n frames.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the payload to filter.
    n : int
        Maximum number of frames to keep (from the bottom of the stack).

    Returns
    -------
    None
        Stores filtered result in filter_fixture["filtered"].

    """
    filter_fixture["filtered"] = typ.cast(
        "StackPayload | ExceptionPayload",
        filter_frames(filter_fixture["payload"], max_depth=n),
    )


@when(
    parsers.parse(
        'I filter with exclude_logging=True, exclude_filenames=["{p}"], max_depth={n:d}'
    )
)
def filter_combined(filter_fixture: FilterFixture, p: str, n: int) -> None:
    """Filter with multiple options: exclude logging, filename pattern, and max depth.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the payload to filter.
    p : str
        Substring pattern to match against frame filenames.
    n : int
        Maximum number of frames to keep (from the bottom of the stack).

    Returns
    -------
    None
        Stores filtered result in filter_fixture["filtered"].

    """
    filter_fixture["filtered"] = typ.cast(
        "StackPayload | ExceptionPayload",
        filter_frames(
            filter_fixture["payload"],
            exclude_logging=True,
            exclude_filenames=[p],
            max_depth=n,
        ),
    )


@when("I get the logging infrastructure patterns")
def get_patterns(filter_fixture: FilterFixture) -> None:
    """Retrieve the list of logging infrastructure patterns.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage.

    Returns
    -------
    None
        Stores patterns in filter_fixture["patterns"].

    """
    filter_fixture["patterns"] = list(get_logging_infrastructure_patterns())


@then(parsers.parse("the filtered payload has {n:d} frame"))
@then(parsers.parse("the filtered payload has {n:d} frames"))
def check_frame_count(filter_fixture: FilterFixture, n: int) -> None:
    """Assert the filtered payload has exactly n frames.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the filtered result.
    n : int
        Expected number of frames.

    Returns
    -------
    None
        Raises AssertionError if frame count does not match.

    """
    frames = _get_frames(filter_fixture)
    assert len(frames) == n, f"Expected {n} frames, got {len(frames)}"


@then(parsers.parse('the filtered frame filename is "{expected}"'))
def check_frame_filename(filter_fixture: FilterFixture, expected: str) -> None:
    """Assert the single filtered frame has the expected filename.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the filtered result.
    expected : str
        Expected filename of the single frame.

    Returns
    -------
    None
        Raises AssertionError if not exactly 1 frame or filename differs.

    """
    frames = _get_frames(filter_fixture)
    assert len(frames) == 1, "Expected exactly 1 frame"
    actual = frames[0]["filename"]
    assert actual == expected, f"Expected filename '{expected}', got '{actual}'"


@then(parsers.parse('the filtered frames do not contain "{pattern}"'))
def check_frames_exclude_pattern(filter_fixture: FilterFixture, pattern: str) -> None:
    """Assert no filtered frame filename contains the pattern.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the filtered result.
    pattern : str
        Substring that should not appear in any frame filename.

    Returns
    -------
    None
        Raises AssertionError if any frame filename contains the pattern.

    """
    frames = _get_frames(filter_fixture)
    for frame in frames:
        assert pattern not in frame["filename"], (
            f"Frame {frame['filename']} should not contain {pattern}"
        )


@then(parsers.parse('the filtered frames are "{f1}", "{f2}"'))
def check_frames_order(filter_fixture: FilterFixture, f1: str, f2: str) -> None:
    """Assert the filtered result has exactly two frames in order.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the filtered result.
    f1 : str
        Expected filename of the first frame.
    f2 : str
        Expected filename of the second frame.

    Returns
    -------
    None
        Raises AssertionError if not exactly 2 frames or order differs.

    """
    frames = _get_frames(filter_fixture)
    assert len(frames) == 2, f"Expected 2 frames, got {len(frames)}"
    assert frames[0]["filename"] == f1, f"First frame should be {f1}"
    assert frames[1]["filename"] == f2, f"Second frame should be {f2}"


@then(parsers.parse("the filtered cause has {n:d} frame"))
@then(parsers.parse("the filtered cause has {n:d} frames"))
def check_cause_frame_count(filter_fixture: FilterFixture, n: int) -> None:
    """Assert the filtered cause exception has exactly n frames.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the filtered exception result.
    n : int
        Expected number of frames in the cause.

    Returns
    -------
    None
        Raises AssertionError if cause frame count does not match.

    """
    frames = _get_frames(filter_fixture, from_cause=True)
    assert len(frames) == n, f"Expected {n} cause frames, got {len(frames)}"


@then(parsers.parse('the filtered cause frame filename is "{expected}"'))
def check_cause_frame_filename(filter_fixture: FilterFixture, expected: str) -> None:
    """Assert the single filtered cause frame has the expected filename.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the filtered exception result.
    expected : str
        Expected filename of the single cause frame.

    Returns
    -------
    None
        Raises AssertionError if not exactly 1 cause frame or filename differs.

    """
    frames = _get_frames(filter_fixture, from_cause=True)
    assert len(frames) == 1, "Expected exactly 1 cause frame"
    actual = frames[0]["filename"]
    assert actual == expected, f"Expected cause filename '{expected}', got '{actual}'"


@then(parsers.parse('the patterns contain "{pattern}"'))
def check_patterns_contain(filter_fixture: FilterFixture, pattern: str) -> None:
    """Assert the logging infrastructure patterns include the given pattern.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared test state storage containing the patterns list.
    pattern : str
        Pattern expected to be in the list.

    Returns
    -------
    None
        Raises AssertionError if pattern is not found.

    """
    patterns = filter_fixture["patterns"]
    assert pattern in patterns, f"Expected {pattern} in patterns: {patterns}"
