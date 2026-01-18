"""BDD step definitions for frame filtering scenarios.

This module provides pytest-bdd step functions that implement the Gherkin
scenarios defined in ``tests/features/frame_filter.feature``. The steps
exercise the ``filter_frames`` and ``get_logging_infrastructure_patterns``
functions from the femtologging package.

Shared types and helpers are imported from ``frame_filter_steps.utils``.

Usage
-----
Run the frame filter scenarios with pytest::

    pytest tests/features/frame_filter.feature -v
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import filter_frames, get_logging_infrastructure_patterns
from tests.frame_filter.conftest import make_exception_payload, make_stack_payload
from tests.steps.frame_filter_steps.utils import (
    FilterFixture,
    _get_frames,
    _parse_filenames,
)

if typ.TYPE_CHECKING:
    from tests.frame_filter.conftest import ExceptionPayload, StackPayload

FEATURES = Path(__file__).resolve().parents[1] / "features"
scenarios(str(FEATURES / "frame_filter.feature"))


@pytest.fixture
def filter_fixture() -> FilterFixture:
    """Provide shared state storage for filter BDD steps.

    Returns
    -------
    FilterFixture
        Empty TypedDict for storing payload, filtered result, and patterns.

    """
    return typ.cast("FilterFixture", {})


# --- Given steps ---


@given(parsers.parse("a stack_info payload with frames from {filenames}"))
def create_stack_payload(filter_fixture: FilterFixture, filenames: str) -> None:
    """Create a stack_info payload from comma-separated filenames.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage for the scenario.
    filenames : str
        Comma-separated quoted filenames (e.g., '"a.py", "b.py"').

    """
    parsed = _parse_filenames(filenames)
    filter_fixture["payload"] = make_stack_payload(parsed)


@given(parsers.parse("an exception payload with frames from {filenames}"))
def create_exception_payload(filter_fixture: FilterFixture, filenames: str) -> None:
    """Create an exception payload from comma-separated filenames.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage for the scenario.
    filenames : str
        Comma-separated quoted filenames (e.g., '"a.py", "b.py"').

    """
    parsed = _parse_filenames(filenames)
    filter_fixture["payload"] = make_exception_payload(parsed, type_name="TestError")


@given(parsers.parse("the exception has a cause with frames from {filenames}"))
def add_cause_to_exception(filter_fixture: FilterFixture, filenames: str) -> None:
    """Add a cause exception to the existing exception payload.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage containing an existing exception payload.
    filenames : str
        Comma-separated quoted filenames for the cause exception frames.

    """
    parsed = _parse_filenames(filenames)
    cause = make_exception_payload(
        parsed, type_name="CauseError", message="cause error"
    )
    typ.cast("ExceptionPayload", filter_fixture["payload"])["cause"] = cause


# --- When steps ---


@when("I filter with exclude_logging=True")
def filter_exclude_logging(filter_fixture: FilterFixture) -> None:
    """Filter the payload excluding logging infrastructure frames.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage containing the payload to filter.

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
        Shared state storage containing the payload to filter.
    pattern : str
        Substring pattern to match against frame filenames.

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
        Shared state storage containing the payload to filter.
    n : int
        Maximum number of frames to retain.

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
    """Filter with exclude logging, filename pattern, and max depth.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage containing the payload to filter.
    p : str
        Substring pattern to match against frame filenames.
    n : int
        Maximum number of frames to retain after exclusions.

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
        Shared state storage to receive the patterns list.

    """
    filter_fixture["patterns"] = list(get_logging_infrastructure_patterns())


# --- Then steps ---


@then(parsers.parse("the filtered payload has {n:d} frame"))
@then(parsers.parse("the filtered payload has {n:d} frames"))
def check_frame_count(filter_fixture: FilterFixture, n: int) -> None:
    """Assert the filtered payload has exactly n frames.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage containing the filtered result.
    n : int
        Expected number of frames.

    """
    frames = _get_frames(filter_fixture)
    assert len(frames) == n, f"Expected {n} frames, got {len(frames)}"


@then(parsers.parse('the filtered frame filename is "{expected}"'))
def check_frame_filename(filter_fixture: FilterFixture, expected: str) -> None:
    """Assert the single filtered frame has the expected filename.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage containing the filtered result.
    expected : str
        Expected filename of the single remaining frame.

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
        Shared state storage containing the filtered result.
    pattern : str
        Substring that must not appear in any frame filename.

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
        Shared state storage containing the filtered result.
    f1 : str
        Expected filename of the first frame.
    f2 : str
        Expected filename of the second frame.

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
        Shared state storage containing the filtered result with cause.
    n : int
        Expected number of frames in the cause exception.

    """
    frames = _get_frames(filter_fixture, from_cause=True)
    assert len(frames) == n, f"Expected {n} cause frames, got {len(frames)}"


@then(parsers.parse('the filtered cause frame filename is "{expected}"'))
def check_cause_frame_filename(filter_fixture: FilterFixture, expected: str) -> None:
    """Assert the single filtered cause frame has the expected filename.

    Parameters
    ----------
    filter_fixture : FilterFixture
        Shared state storage containing the filtered result with cause.
    expected : str
        Expected filename of the single remaining cause frame.

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
        Shared state storage containing the patterns list.
    pattern : str
        Pattern string expected to be in the list.

    """
    patterns = filter_fixture["patterns"]
    assert pattern in patterns, f"Expected {pattern} in patterns: {patterns}"
