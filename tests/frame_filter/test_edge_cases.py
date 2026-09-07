"""Unit tests for edge cases and error handling in frame filtering."""

from __future__ import annotations

import pytest

from femtologging import filter_frames

from .conftest import StackPayload, assert_frame_filenames


def test_edge_missing_frames_key() -> None:
    """Payload without frames key should filter to an empty stack."""
    payload: StackPayload = {"schema_version": 1}

    result = filter_frames(payload)

    assert_frame_filenames(result, [], "payload without a frames key")


@pytest.mark.parametrize(
    ("missing_field", "frame_data"),
    [
        pytest.param(
            "filename",
            {"lineno": 1, "function": "test"},
            id="missing-filename",
        ),
        pytest.param(
            "lineno",
            {"filename": "a.py", "function": "test"},
            id="missing-lineno",
        ),
        pytest.param(
            "function",
            {"filename": "a.py", "lineno": 1},
            id="missing-function",
        ),
    ],
)
def test_edge_invalid_frame_missing_required_field(
    missing_field: str,
    frame_data: dict[str, object],
) -> None:
    """Frame missing a required field should raise TypeError naming that field."""
    payload: dict[str, object] = {
        "schema_version": 1,
        "frames": [frame_data],
    }

    with pytest.raises(TypeError, match=missing_field):
        filter_frames(payload)


@pytest.mark.parametrize(
    ("frames_value", "expected_error"),
    [
        pytest.param("not a list", "must be a list", id="frames-not-a-list"),
        pytest.param(["not a dict"], "must be a dict", id="frame-not-a-dict"),
        pytest.param(
            [{"filename": "a.py", "lineno": 1, "function": "test", "end_lineno": "x"}],
            "wrong type",
            id="optional-field-wrong-type",
        ),
    ],
)
def test_edge_malformed_frames_raise_type_error(
    frames_value: object,
    expected_error: str,
) -> None:
    """Malformed frame payloads should raise TypeError with a descriptive message."""
    payload: dict[str, object] = {
        "schema_version": 1,
        "frames": frames_value,
    }

    with pytest.raises(TypeError, match=expected_error):
        filter_frames(payload)
