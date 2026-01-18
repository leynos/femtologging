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


@pytest.mark.parametrize(
    ("missing_field", "frame_data"),
    [
        ("filename", {"lineno": 1, "function": "test"}),
        ("lineno", {"filename": "a.py", "function": "test"}),
        ("function", {"filename": "a.py", "lineno": 1}),
    ],
)
def test_edge_invalid_frame_missing_required_field(
    missing_field: str,
    frame_data: dict[str, object],
) -> None:
    """Frame missing a required field should raise TypeError."""
    payload: dict[str, object] = {
        "schema_version": 1,
        "frames": [frame_data],
    }

    with pytest.raises(TypeError, match=missing_field):
        filter_frames(payload)


@pytest.mark.parametrize(
    ("frames_value", "expected_error"),
    [
        ("not a list", "must be a list"),
        (["not a dict"], "must be a dict"),
    ],
)
def test_edge_invalid_frames_shape(
    frames_value: object,
    expected_error: str,
) -> None:
    """Malformed frames should raise TypeError with descriptive message."""
    payload: dict[str, object] = {
        "schema_version": 1,
        "frames": frames_value,
    }

    with pytest.raises(TypeError, match=expected_error):
        filter_frames(payload)


def test_edge_optional_field_wrong_type() -> None:
    """Optional field with wrong type should raise TypeError."""
    payload: dict[str, object] = {
        "schema_version": 1,
        "frames": [
            {"filename": "a.py", "lineno": 1, "function": "test", "end_lineno": "wrong"}
        ],
    }

    with pytest.raises(TypeError, match="wrong type"):
        filter_frames(payload)
