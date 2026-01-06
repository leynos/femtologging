"""Unit tests for frame filtering utilities."""

from __future__ import annotations

import pytest

from femtologging import filter_frames, get_logging_infrastructure_patterns


def make_stack_payload(filenames: list[str]) -> dict:
    """Create a stack_info payload dict with the given filenames."""
    frames = [
        {"filename": fn, "lineno": i + 1, "function": f"func_{i}"}
        for i, fn in enumerate(filenames)
    ]
    return {"schema_version": 1, "frames": frames}


def make_exception_payload(
    filenames: list[str],
    type_name: str = "ValueError",
    message: str = "test error",
) -> dict:
    """Create an exc_info payload dict with the given filenames."""
    payload = make_stack_payload(filenames)
    payload["type_name"] = type_name
    payload["message"] = message
    return payload


# Tests for filtering stack_info payloads


def test_stack_exclude_logging_infrastructure() -> None:
    """Logging infrastructure frames should be excluded."""
    payload = make_stack_payload([
        "myapp/main.py",
        "femtologging/__init__.py",
        "logging/__init__.py",
    ])

    result = filter_frames(payload, exclude_logging=True)

    assert len(result["frames"]) == 1
    assert result["frames"][0]["filename"] == "myapp/main.py"


def test_stack_exclude_filenames_single_pattern() -> None:
    """Single filename pattern should exclude matching frames."""
    payload = make_stack_payload([
        "myapp/main.py",
        ".venv/lib/requests.py",
        "myapp/utils.py",
    ])

    result = filter_frames(payload, exclude_filenames=[".venv/"])

    assert len(result["frames"]) == 2
    filenames = [f["filename"] for f in result["frames"]]
    assert ".venv/lib/requests.py" not in filenames


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

    assert len(result["frames"]) == 1
    assert result["frames"][0]["filename"] == "myapp/main.py"


def test_stack_exclude_functions() -> None:
    """Function patterns should exclude matching frames."""
    payload = make_stack_payload(["a.py", "b.py", "c.py"])
    payload["frames"][1]["function"] = "_internal_helper"

    result = filter_frames(payload, exclude_functions=["_internal"])

    assert len(result["frames"]) == 2
    functions = [f["function"] for f in result["frames"]]
    assert "_internal_helper" not in functions


def test_stack_max_depth_limits_frames() -> None:
    """Max depth should keep only the most recent frames."""
    payload = make_stack_payload(["a.py", "b.py", "c.py", "d.py", "e.py"])

    result = filter_frames(payload, max_depth=2)

    assert len(result["frames"]) == 2
    # Should be the last 2 frames (d.py, e.py)
    assert result["frames"][0]["filename"] == "d.py"
    assert result["frames"][1]["filename"] == "e.py"


def test_stack_max_depth_under_limit() -> None:
    """Max depth greater than frame count should return all frames."""
    payload = make_stack_payload(["a.py", "b.py"])

    result = filter_frames(payload, max_depth=10)

    assert len(result["frames"]) == 2


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
    assert len(result["frames"]) == 2
    assert result["frames"][0]["filename"] == "myapp/api.py"
    assert result["frames"][1]["filename"] == "myapp/handler.py"


def test_stack_preserves_schema_version() -> None:
    """Schema version should be preserved in the result."""
    payload = make_stack_payload(["a.py"])
    payload["schema_version"] = 42

    result = filter_frames(payload, max_depth=10)

    assert result["schema_version"] == 42


def test_stack_empty_frames() -> None:
    """Empty frames list should produce empty result."""
    payload: dict = {"schema_version": 1, "frames": []}

    result = filter_frames(payload, exclude_logging=True)

    assert result["frames"] == []


def test_stack_no_filters_returns_copy() -> None:
    """No filters should return all frames."""
    payload = make_stack_payload(["a.py", "b.py", "c.py"])

    result = filter_frames(payload)

    assert len(result["frames"]) == 3


# Tests for filtering exc_info payloads


def test_exc_detects_exception_payload() -> None:
    """Exception payloads should be detected by type_name and message."""
    payload = make_exception_payload(
        ["myapp/main.py", "femtologging/__init__.py"],
    )

    result = filter_frames(payload, exclude_logging=True)

    # Should preserve exception fields
    assert result["type_name"] == "ValueError"
    assert result["message"] == "test error"
    assert len(result["frames"]) == 1


def test_exc_filters_cause_chain() -> None:
    """Cause chain should be recursively filtered."""
    cause = make_exception_payload(
        ["cause.py", "femtologging/__init__.py"],
        type_name="IOError",
        message="cause error",
    )
    payload = make_exception_payload(
        ["main.py", "logging/__init__.py"],
    )
    payload["cause"] = cause

    result = filter_frames(payload, exclude_logging=True)

    # Main frames filtered
    assert len(result["frames"]) == 1
    assert result["frames"][0]["filename"] == "main.py"

    # Cause frames also filtered
    assert "cause" in result
    assert len(result["cause"]["frames"]) == 1
    assert result["cause"]["frames"][0]["filename"] == "cause.py"


def test_exc_filters_context_chain() -> None:
    """Context chain should be recursively filtered."""
    context = make_exception_payload(
        ["context.py", "femtologging/__init__.py"],
        type_name="KeyError",
        message="context error",
    )
    payload = make_exception_payload(["main.py"])
    payload["context"] = context

    result = filter_frames(payload, exclude_logging=True)

    assert "context" in result
    assert len(result["context"]["frames"]) == 1
    assert result["context"]["frames"][0]["filename"] == "context.py"


def test_exc_filters_exception_group() -> None:
    """Exception group members should be recursively filtered."""
    exc1 = make_exception_payload(
        ["exc1.py", "femtologging/__init__.py"],
        type_name="ValueError",
        message="error 1",
    )
    exc2 = make_exception_payload(
        ["exc2.py", "logging/__init__.py"],
        type_name="TypeError",
        message="error 2",
    )
    payload = make_exception_payload(
        ["group.py"],
        type_name="ExceptionGroup",
        message="multiple errors",
    )
    payload["exceptions"] = [exc1, exc2]

    result = filter_frames(payload, exclude_logging=True)

    assert len(result["exceptions"]) == 2
    assert len(result["exceptions"][0]["frames"]) == 1
    assert result["exceptions"][0]["frames"][0]["filename"] == "exc1.py"
    assert len(result["exceptions"][1]["frames"]) == 1
    assert result["exceptions"][1]["frames"][0]["filename"] == "exc2.py"


def test_exc_preserves_exception_fields() -> None:
    """All exception fields should be preserved."""
    payload = make_exception_payload(["main.py"])
    payload["module"] = "myapp.errors"
    payload["args_repr"] = ["'key'"]
    payload["notes"] = ["check the input"]
    payload["suppress_context"] = True

    result = filter_frames(payload)

    assert result["type_name"] == "ValueError"
    assert result["message"] == "test error"
    assert result["module"] == "myapp.errors"
    assert result["args_repr"] == ["'key'"]
    assert result["notes"] == ["check the input"]
    assert result["suppress_context"] is True


# Tests for get_logging_infrastructure_patterns


def test_patterns_returns_expected() -> None:
    """Should return the default logging infrastructure patterns."""
    patterns = get_logging_infrastructure_patterns()

    assert "femtologging" in patterns
    assert "logging/__init__" in patterns
    assert "_femtologging_rs" in patterns


def test_patterns_match_expected_files() -> None:
    """Patterns should match expected logging infrastructure files."""
    patterns = get_logging_infrastructure_patterns()

    # Test that patterns work as expected (substring matching)
    test_filenames = [
        ("femtologging/__init__.py", True),
        ("_femtologging_rs.cpython-311.so", True),
        ("logging/__init__.py", True),
        ("logging/config.py", True),
        ("myapp/main.py", False),
        ("myapp/logging_utils.py", False),  # "logging" is not a pattern
    ]

    for filename, should_match in test_filenames:
        matches = any(p in filename for p in patterns)
        assert matches == should_match, f"{filename}: expected {should_match}"


# Tests for edge cases and error handling


def test_edge_missing_frames_key() -> None:
    """Payload without frames key should return empty frames."""
    payload: dict = {"schema_version": 1}

    result = filter_frames(payload)

    assert result.get("frames", []) == []


def test_edge_preserves_frame_details() -> None:
    """Frame details like source_line should be preserved."""
    payload = make_stack_payload(["a.py"])
    payload["frames"][0]["source_line"] = "    x = 42"
    payload["frames"][0]["colno"] = 5
    payload["frames"][0]["end_colno"] = 10

    result = filter_frames(payload)

    assert result["frames"][0]["source_line"] == "    x = 42"
    assert result["frames"][0]["colno"] == 5
    assert result["frames"][0]["end_colno"] == 10


def test_edge_invalid_frame_missing_filename() -> None:
    """Frame missing required filename should raise TypeError."""
    payload: dict = {
        "schema_version": 1,
        "frames": [{"lineno": 1, "function": "test"}],
    }

    with pytest.raises(TypeError, match="filename"):
        filter_frames(payload)


def test_edge_invalid_frame_missing_lineno() -> None:
    """Frame missing required lineno should raise TypeError."""
    payload: dict = {
        "schema_version": 1,
        "frames": [{"filename": "a.py", "function": "test"}],
    }

    with pytest.raises(TypeError, match="lineno"):
        filter_frames(payload)


def test_edge_invalid_frame_missing_function() -> None:
    """Frame missing required function should raise TypeError."""
    payload: dict = {
        "schema_version": 1,
        "frames": [{"filename": "a.py", "lineno": 1}],
    }

    with pytest.raises(TypeError, match="function"):
        filter_frames(payload)
