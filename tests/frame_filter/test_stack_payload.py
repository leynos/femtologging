"""Unit tests for stack payload filtering."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import filter_frames

from .conftest import (
    StackPayload,
    assert_frame_filenames,
    assert_frame_functions,
    make_stack_payload,
)

_LOGGING_INFRASTRUCTURE_FILENAMES = [
    "myapp/main.py",
    "femtologging/__init__.py",
    "logging/__init__.py",
]


def test_stack_exclude_logging_infrastructure() -> None:
    """Logging infrastructure frames should be excluded."""
    payload = make_stack_payload(_LOGGING_INFRASTRUCTURE_FILENAMES)

    result = filter_frames(payload, exclude_logging=True)

    assert_frame_filenames(result, ["myapp/main.py"], "exclude_logging")


@pytest.mark.parametrize(
    ("filenames", "exclude_filenames", "expected"),
    [
        pytest.param(
            ["myapp/main.py", ".venv/lib/requests.py", "myapp/utils.py"],
            [".venv/"],
            ["myapp/main.py", "myapp/utils.py"],
            id="single-pattern",
        ),
        pytest.param(
            ["myapp/main.py", ".venv/lib/foo.py", "site-packages/bar.py"],
            [".venv/", "site-packages/"],
            ["myapp/main.py"],
            id="multiple-patterns",
        ),
        pytest.param(
            ["myapp/main.py", "myapp/utils.py"],
            ["no-such-directory/"],
            ["myapp/main.py", "myapp/utils.py"],
            id="no-pattern-matches",
        ),
        pytest.param(
            [".venv/lib/foo.py", ".venv/lib/bar.py"],
            [".venv/"],
            [],
            id="every-frame-excluded",
        ),
    ],
)
def test_stack_exclude_filenames(
    filenames: list[str],
    exclude_filenames: list[str],
    expected: list[str],
) -> None:
    """Filename patterns should exclude every frame whose path contains them."""
    payload = make_stack_payload(filenames)

    result = filter_frames(payload, exclude_filenames=exclude_filenames)

    assert_frame_filenames(
        result,
        expected,
        f"exclude_filenames={exclude_filenames}",
    )


def test_stack_exclude_functions() -> None:
    """Function patterns should exclude matching frames."""
    payload = make_stack_payload(["a.py", "b.py", "c.py"])
    payload["frames"][1]["function"] = "_internal_helper"

    result = filter_frames(payload, exclude_functions=["_internal"])

    assert_frame_functions(result, ["func_0", "func_2"], "exclude_functions")


@pytest.mark.parametrize(
    ("filenames", "max_depth", "expected"),
    [
        pytest.param(
            ["a.py", "b.py", "c.py", "d.py", "e.py"],
            2,
            ["d.py", "e.py"],
            id="keeps-most-recent-frames",
        ),
        pytest.param(
            ["a.py", "b.py"],
            10,
            ["a.py", "b.py"],
            id="limit-above-frame-count",
        ),
        pytest.param(
            ["a.py", "b.py", "c.py"],
            3,
            ["a.py", "b.py", "c.py"],
            id="limit-equals-frame-count",
        ),
        pytest.param(["a.py", "b.py"], 1, ["b.py"], id="single-frame-limit"),
    ],
)
def test_stack_max_depth_keeps_trailing_frames(
    filenames: list[str],
    max_depth: int,
    expected: list[str],
) -> None:
    """Max depth should keep the most recent frames, in original order."""
    payload = make_stack_payload(filenames)

    result = filter_frames(payload, max_depth=max_depth)

    assert_frame_filenames(result, expected, f"max_depth={max_depth}")


def test_stack_combined_filters() -> None:
    """Multiple filters should be applied in sequence."""
    payload = make_stack_payload([
        "outer.py",
        ".venv/lib/requests.py",
        "myapp/api.py",
        "femtologging/__init__.py",
        "myapp/handler.py",
    ])

    result = filter_frames(
        payload,
        exclude_logging=True,
        exclude_filenames=[".venv/"],
        max_depth=2,
    )

    # After exclude_logging: outer.py, .venv/..., myapp/api.py, myapp/handler.py
    # After exclude_filenames: outer.py, myapp/api.py, myapp/handler.py
    # After max_depth=2: myapp/api.py, myapp/handler.py
    assert_frame_filenames(
        result,
        ["myapp/api.py", "myapp/handler.py"],
        "exclude_logging + exclude_filenames + max_depth",
    )


def test_stack_preserves_schema_version() -> None:
    """Schema version should be preserved in the result."""
    payload: StackPayload = make_stack_payload(["a.py"])
    payload["schema_version"] = 42

    result = filter_frames(payload, max_depth=10)

    assert result["schema_version"] == 42, "schema version should be preserved"


def test_stack_empty_frames() -> None:
    """Empty frames list should omit frames key to match serialization semantics."""
    payload: StackPayload = {"schema_version": 1, "frames": []}

    result = filter_frames(payload, exclude_logging=True)

    # Empty frames omitted to match serde skip_serializing_if = "Vec::is_empty"
    assert "frames" not in result, (
        f"empty frames should be omitted from the payload, got {result}"
    )


def test_stack_no_filters_returns_copy() -> None:
    """No filters should return all frames unchanged."""
    filenames = ["a.py", "b.py", "c.py"]
    payload = make_stack_payload(filenames)

    result = filter_frames(payload)

    assert_frame_filenames(result, filenames, "no filters")


def test_stack_preserves_extra_keys() -> None:
    """Stack payload should preserve all keys, not just schema_version and frames."""
    payload = make_stack_payload(["a.py", "b.py"])
    # Cast around the TypedDict's closed key set so the test can add extras.
    payload_dict = typ.cast("dict[str, object]", payload)
    extras: dict[str, object] = {
        "thread_id": 12345,
        "process_id": 67890,
        "custom_field": "preserved",
    }
    payload_dict.update(extras)

    result = filter_frames(payload_dict, max_depth=1)

    for key, expected in extras.items():
        assert result[key] == expected, (
            f"{key} should survive filtering: expected {expected!r}, "
            f"got {result.get(key)!r}"
        )


def test_stack_preserves_frame_details() -> None:
    """Optional per-frame details should survive filtering."""
    payload = make_stack_payload(["a.py", "b.py"])
    details = {
        "source_line": "    x = 42",
        "colno": 5,
        "end_colno": 10,
        "locals": {"x": "42", "y": "hello"},
    }
    # Cast around the TypedDict's closed key set so the details can be applied
    # from a table rather than one statement per optional field.
    typ.cast("dict[str, object]", payload["frames"][0]).update(details)

    result = filter_frames(payload)

    for key, expected in details.items():
        assert result["frames"][0][key] == expected, (
            f"frame detail {key} should be preserved: expected {expected!r}, "
            f"got {result['frames'][0].get(key)!r}"
        )
