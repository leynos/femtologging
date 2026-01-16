"""Unit tests for stack payload filtering."""

from __future__ import annotations

from femtologging import filter_frames

from .conftest import StackPayload, make_stack_payload


def test_stack_exclude_logging_infrastructure() -> None:
    """Logging infrastructure frames should be excluded."""
    payload = make_stack_payload([
        "myapp/main.py",
        "femtologging/__init__.py",
        "logging/__init__.py",
    ])

    result = filter_frames(payload, exclude_logging=True)

    assert len(result["frames"]) == 1, "expected 1 frame after filtering"
    assert result["frames"][0]["filename"] == "myapp/main.py", "wrong frame filename"


def test_stack_exclude_filenames_single_pattern() -> None:
    """Single filename pattern should exclude matching frames."""
    payload = make_stack_payload([
        "myapp/main.py",
        ".venv/lib/requests.py",
        "myapp/utils.py",
    ])

    result = filter_frames(payload, exclude_filenames=[".venv/"])

    assert len(result["frames"]) == 2, "expected 2 frames after filtering"
    filenames = [f["filename"] for f in result["frames"]]
    assert ".venv/lib/requests.py" not in filenames, "venv frame should be excluded"


def test_stack_exclude_filenames_multiple_patterns() -> None:
    """Multiple filename patterns should exclude all matching frames."""
    payload = make_stack_payload([
        "myapp/main.py",
        ".venv/lib/foo.py",
        "site-packages/bar.py",
    ])

    result = filter_frames(
        payload,
        exclude_filenames=[".venv/", "site-packages/"],
    )

    assert len(result["frames"]) == 1, "expected 1 frame after filtering"
    assert result["frames"][0]["filename"] == "myapp/main.py", "wrong frame filename"


def test_stack_exclude_functions() -> None:
    """Function patterns should exclude matching frames."""
    payload = make_stack_payload(["a.py", "b.py", "c.py"])
    payload["frames"][1]["function"] = "_internal_helper"

    result = filter_frames(payload, exclude_functions=["_internal"])

    assert len(result["frames"]) == 2, "expected 2 frames after filtering"
    functions = [f["function"] for f in result["frames"]]
    assert "_internal_helper" not in functions, "internal function should be excluded"


def test_stack_max_depth_limits_frames() -> None:
    """Max depth should keep only the most recent frames."""
    payload = make_stack_payload(["a.py", "b.py", "c.py", "d.py", "e.py"])

    result = filter_frames(payload, max_depth=2)

    assert len(result["frames"]) == 2, "expected 2 frames after limiting"
    # Should be the last 2 frames (d.py, e.py)
    assert result["frames"][0]["filename"] == "d.py", "first frame should be d.py"
    assert result["frames"][1]["filename"] == "e.py", "second frame should be e.py"


def test_stack_max_depth_under_limit() -> None:
    """Max depth greater than frame count should return all frames."""
    payload = make_stack_payload(["a.py", "b.py"])

    result = filter_frames(payload, max_depth=10)

    assert len(result["frames"]) == 2, "all frames should be returned"


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
    assert len(result["frames"]) == 2, "expected 2 frames after combined filters"
    assert result["frames"][0]["filename"] == "myapp/api.py", "first frame mismatch"
    assert result["frames"][1]["filename"] == "myapp/handler.py", (
        "second frame mismatch"
    )


def test_stack_preserves_schema_version() -> None:
    """Schema version should be preserved in the result."""
    payload: StackPayload = make_stack_payload(["a.py"])
    payload["schema_version"] = 42

    result = filter_frames(payload, max_depth=10)

    assert result["schema_version"] == 42, "schema version should be preserved"


def test_stack_empty_frames() -> None:
    """Empty frames list should omit frames key to match serialisation semantics."""
    payload: StackPayload = {"schema_version": 1, "frames": []}

    result = filter_frames(payload, exclude_logging=True)

    # Empty frames omitted to match serde skip_serializing_if = "Vec::is_empty"
    assert "frames" not in result, "empty frames should be omitted"


def test_stack_no_filters_returns_copy() -> None:
    """No filters should return all frames."""
    payload = make_stack_payload(["a.py", "b.py", "c.py"])

    result = filter_frames(payload)

    assert len(result["frames"]) == 3, "all frames should be returned"


def test_stack_preserves_extra_keys() -> None:
    """Stack payload should preserve all keys, not just schema_version and frames."""
    payload = make_stack_payload(["a.py", "b.py"])
    payload["thread_id"] = 12345  # type: ignore[typeddict-unknown-key]
    payload["process_id"] = 67890  # type: ignore[typeddict-unknown-key]
    payload["custom_field"] = "preserved"  # type: ignore[typeddict-unknown-key]

    result = filter_frames(payload, max_depth=1)

    assert result["thread_id"] == 12345, "thread_id should be preserved"
    assert result["process_id"] == 67890, "process_id should be preserved"
    assert result["custom_field"] == "preserved", "custom_field should be preserved"


def test_frame_locals_preserved() -> None:
    """Frame locals should be preserved after filtering."""
    payload = make_stack_payload(["a.py", "b.py"])
    payload["frames"][0]["locals"] = {"x": "42", "y": "hello"}

    result = filter_frames(payload)

    assert result["frames"][0]["locals"] == {"x": "42", "y": "hello"}, (
        "locals should be preserved"
    )
