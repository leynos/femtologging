"""BDD steps for frame filtering scenarios."""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import filter_frames, get_logging_infrastructure_patterns


class FrameDict(typ.TypedDict, total=False):
    """Structure for a stack frame."""

    filename: str
    lineno: int
    function: str
    end_lineno: int
    colno: int
    end_colno: int
    source_line: str
    locals: dict[str, str]


class StackPayload(typ.TypedDict, total=False):
    """Structure for a stack_info payload."""

    schema_version: int
    frames: list[FrameDict]


class ExceptionPayload(StackPayload, total=False):
    """Structure for an exc_info payload (extends StackPayload)."""

    type_name: str
    message: str
    module: str
    args_repr: list[str]
    notes: list[str]
    suppress_context: bool
    cause: ExceptionPayload
    context: ExceptionPayload
    exceptions: list[ExceptionPayload]


class FilterFixture(typ.TypedDict, total=False):
    """State storage for filter fixture."""

    payload: dict[str, typ.Any]
    filtered: dict[str, typ.Any]
    patterns: list[str]


FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "frame_filter.feature"))


def _make_frame(filename: str, lineno: int) -> dict[str, typ.Any]:
    """Create a frame dict."""
    return {
        "filename": filename,
        "lineno": lineno,
        "function": f"func_{lineno}",
    }


def _make_stack_payload(filenames: list[str]) -> dict[str, typ.Any]:
    """Create a stack_info payload with given filenames."""
    return {
        "schema_version": 1,
        "frames": [_make_frame(fn, i + 1) for i, fn in enumerate(filenames)],
    }


def _make_exception_payload(filenames: list[str]) -> dict[str, typ.Any]:
    """Create an exc_info payload with given filenames."""
    return {
        "schema_version": 1,
        "frames": [_make_frame(fn, i + 1) for i, fn in enumerate(filenames)],
        "type_name": "TestError",
        "message": "test error",
    }


def _parse_filenames(filenames_str: str) -> list[str]:
    """Parse a comma-separated list of quoted filenames."""
    # Input: '"a.py", "b.py", "c.py"'
    # Output: ['a.py', 'b.py', 'c.py']
    return [f.strip().strip('"') for f in filenames_str.split(",")]


@pytest.fixture
def filter_fixture() -> FilterFixture:
    """Storage for filter state."""
    return {}


@given(parsers.parse("a stack_info payload with frames from {filenames}"))
def create_stack_payload(filter_fixture: FilterFixture, filenames: str) -> None:
    parsed = _parse_filenames(filenames)
    filter_fixture["payload"] = _make_stack_payload(parsed)


@given(parsers.parse("an exception payload with frames from {filenames}"))
def create_exception_payload(filter_fixture: FilterFixture, filenames: str) -> None:
    parsed = _parse_filenames(filenames)
    filter_fixture["payload"] = _make_exception_payload(parsed)


@given(parsers.parse("the exception has a cause with frames from {filenames}"))
def add_cause_to_exception(filter_fixture: FilterFixture, filenames: str) -> None:
    parsed = _parse_filenames(filenames)
    cause = _make_exception_payload(parsed)
    cause["type_name"] = "CauseError"
    cause["message"] = "cause error"
    filter_fixture["payload"]["cause"] = cause


@when("I filter with exclude_logging=True")
def filter_exclude_logging(filter_fixture: FilterFixture) -> None:
    filter_fixture["filtered"] = filter_frames(
        filter_fixture["payload"], exclude_logging=True
    )


@when(parsers.parse('I filter with exclude_filenames=["{pattern}"]'))
def filter_exclude_filenames(filter_fixture: FilterFixture, pattern: str) -> None:
    filter_fixture["filtered"] = filter_frames(
        filter_fixture["payload"], exclude_filenames=[pattern]
    )


@when(parsers.parse("I filter with max_depth={n:d}"))
def filter_max_depth(filter_fixture: FilterFixture, n: int) -> None:
    filter_fixture["filtered"] = filter_frames(filter_fixture["payload"], max_depth=n)


@when(
    parsers.parse(
        'I filter with exclude_logging=True, exclude_filenames=["{p}"], max_depth={n:d}'
    )
)
def filter_combined(filter_fixture: FilterFixture, p: str, n: int) -> None:
    filter_fixture["filtered"] = filter_frames(
        filter_fixture["payload"],
        exclude_logging=True,
        exclude_filenames=[p],
        max_depth=n,
    )


@when("I get the logging infrastructure patterns")
def get_patterns(filter_fixture: FilterFixture) -> None:
    filter_fixture["patterns"] = list(get_logging_infrastructure_patterns())


@then(parsers.parse("the filtered payload has {n:d} frame"))
@then(parsers.parse("the filtered payload has {n:d} frames"))
def check_frame_count(filter_fixture: FilterFixture, n: int) -> None:
    frames = filter_fixture["filtered"].get("frames", [])
    assert len(frames) == n, f"Expected {n} frames, got {len(frames)}"


@then(parsers.parse('the filtered frame filename is "{expected}"'))
def check_frame_filename(filter_fixture: FilterFixture, expected: str) -> None:
    frames = filter_fixture["filtered"]["frames"]
    assert len(frames) == 1, "Expected exactly 1 frame"
    assert frames[0]["filename"] == expected


@then(parsers.parse('the filtered frames do not contain "{pattern}"'))
def check_frames_exclude_pattern(filter_fixture: FilterFixture, pattern: str) -> None:
    frames = filter_fixture["filtered"]["frames"]
    for frame in frames:
        assert pattern not in frame["filename"], (
            f"Frame {frame['filename']} should not contain {pattern}"
        )


@then(parsers.parse('the filtered frames are "{f1}", "{f2}"'))
def check_frames_order(filter_fixture: FilterFixture, f1: str, f2: str) -> None:
    frames = filter_fixture["filtered"]["frames"]
    assert len(frames) == 2, f"Expected 2 frames, got {len(frames)}"
    assert frames[0]["filename"] == f1, f"First frame should be {f1}"
    assert frames[1]["filename"] == f2, f"Second frame should be {f2}"


@then(parsers.parse("the filtered cause has {n:d} frame"))
@then(parsers.parse("the filtered cause has {n:d} frames"))
def check_cause_frame_count(filter_fixture: FilterFixture, n: int) -> None:
    cause = filter_fixture["filtered"].get("cause", {})
    frames = cause.get("frames", [])
    assert len(frames) == n, f"Expected {n} cause frames, got {len(frames)}"


@then(parsers.parse('the filtered cause frame filename is "{expected}"'))
def check_cause_frame_filename(filter_fixture: FilterFixture, expected: str) -> None:
    cause = filter_fixture["filtered"]["cause"]
    frames = cause["frames"]
    assert len(frames) == 1, "Expected exactly 1 cause frame"
    assert frames[0]["filename"] == expected


@then(parsers.parse('the patterns contain "{pattern}"'))
def check_patterns_contain(filter_fixture: FilterFixture, pattern: str) -> None:
    patterns = filter_fixture["patterns"]
    assert pattern in patterns, f"Expected {pattern} in patterns: {patterns}"
