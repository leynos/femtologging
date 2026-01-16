"""Unit tests for edge cases and error handling in frame filtering."""

from __future__ import annotations

import pytest

from femtologging import filter_frames

from .conftest import StackPayload, make_stack_payload


def test_edge_missing_frames_key() -> None:
    """Payload without frames key should return empty frames."""
    payload: StackPayload = {"schema_version": 1}

    result = filter_frames(payload)

    assert result.get("frames", []) == [], "missing frames should return empty"


def test_edge_preserves_frame_details() -> None:
    """Frame details like source_line should be preserved."""
    payload = make_stack_payload(["a.py"])
    payload["frames"][0]["source_line"] = "    x = 42"
    payload["frames"][0]["colno"] = 5
    payload["frames"][0]["end_colno"] = 10

    result = filter_frames(payload)

    assert result["frames"][0]["source_line"] == "    x = 42", (
        "source_line should be preserved"
    )
    assert result["frames"][0]["colno"] == 5, "colno should be preserved"
    assert result["frames"][0]["end_colno"] == 10, "end_colno should be preserved"


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


def test_edge_frames_not_list() -> None:
    """Frames that is not a list should raise TypeError."""
    payload: dict = {
        "schema_version": 1,
        "frames": "not a list",
    }

    with pytest.raises(TypeError, match="must be a list"):
        filter_frames(payload)


def test_edge_frame_not_dict() -> None:
    """Frame that is not a dict should raise TypeError."""
    payload: dict = {
        "schema_version": 1,
        "frames": ["not a dict"],
    }

    with pytest.raises(TypeError, match="must be a dict"):
        filter_frames(payload)


def test_edge_optional_field_wrong_type() -> None:
    """Optional field with wrong type should raise TypeError."""
    payload: dict = {
        "schema_version": 1,
        "frames": [
            {"filename": "a.py", "lineno": 1, "function": "test", "end_lineno": "wrong"}
        ],
    }

    with pytest.raises(TypeError, match="wrong type"):
        filter_frames(payload)
