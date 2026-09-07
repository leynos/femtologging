"""Unit tests for exception payload filtering."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import filter_frames

from .conftest import (
    FilteredPayload,
    assert_frame_filenames,
    assert_frame_functions,
    make_exception_payload,
)

# Depth of the synthetic cause chain used to prove recursion is not bounded by
# an accidental limit and does not blow the interpreter stack.
_DEEP_CHAIN_LENGTH = 100


def _assert_exception_identity(
    payload: FilteredPayload,
    type_name: str,
    message: str,
    context: str,
) -> None:
    """Assert that filtering preserved an exception's type name and message."""
    assert (payload.get("type_name"), payload.get("message")) == (type_name, message), (
        f"{context}: expected exception {type_name}({message!r}), got "
        f"{payload.get('type_name')}({payload.get('message')!r})"
    )


def test_exc_detects_exception_payload() -> None:
    """Exception payloads should be filtered while keeping their identity."""
    payload = make_exception_payload(
        ["myapp/main.py", "femtologging/__init__.py"],
    )

    result = filter_frames(payload, exclude_logging=True)

    _assert_exception_identity(result, "ValueError", "test error", "filtered exception")
    assert_frame_filenames(result, ["myapp/main.py"], "exception frames")


@pytest.mark.parametrize("link", ["cause", "context"])
def test_exc_filters_linked_exception(link: str) -> None:
    """Cause and context links should be recursively filtered."""
    linked = make_exception_payload(
        [f"{link}.py", "femtologging/__init__.py"],
        type_name="OSError",
        message=f"{link} error",
    )
    payload = make_exception_payload(["main.py", "logging/__init__.py"])
    # Cast around the TypedDict's closed key set so one test can drive both the
    # "cause" and "context" links.
    typ.cast("dict[str, object]", payload)[link] = linked

    result = filter_frames(payload, exclude_logging=True)

    assert_frame_filenames(result, ["main.py"], "outer exception frames")
    assert link in result, f"{link} link should survive filtering, got {result}"
    assert_frame_filenames(result[link], [f"{link}.py"], f"{link} frames")
    _assert_exception_identity(
        result[link],
        "OSError",
        f"{link} error",
        f"{link} identity",
    )


def test_exc_filters_exception_group() -> None:
    """Exception group members should be recursively filtered."""
    members = [
        make_exception_payload(
            ["exc1.py", "femtologging/__init__.py"],
            type_name="ValueError",
            message="error 1",
        ),
        make_exception_payload(
            ["exc2.py", "logging/__init__.py"],
            type_name="TypeError",
            message="error 2",
        ),
    ]
    payload = make_exception_payload(
        ["group.py"],
        type_name="ExceptionGroup",
        message="multiple errors",
    )
    payload["exceptions"] = members

    result = filter_frames(payload, exclude_logging=True)

    assert len(result["exceptions"]) == 2, (
        f"both group members should survive filtering, got {result['exceptions']}"
    )
    for index, expected_filename in enumerate(["exc1.py", "exc2.py"]):
        assert_frame_filenames(
            result["exceptions"][index],
            [expected_filename],
            f"exception group member {index}",
        )


def test_exc_preserves_exception_fields() -> None:
    """All exception fields should be preserved."""
    payload = make_exception_payload(["main.py"])
    extras: dict[str, object] = {
        "module": "myapp.errors",
        "args_repr": ["'key'"],
        "notes": ["check the input"],
        "suppress_context": True,
    }
    # Cast around the TypedDict's closed key set so the optional exception
    # fields can be applied from a table.
    typ.cast("dict[str, object]", payload).update(extras)

    result = filter_frames(payload)

    _assert_exception_identity(result, "ValueError", "test error", "unfiltered payload")
    for key, expected in extras.items():
        assert result[key] == expected, (
            f"{key} should be preserved: expected {expected!r}, got {result.get(key)!r}"
        )


def test_exc_exclude_functions() -> None:
    """Function patterns should exclude matching frames in exception payloads."""
    payload = make_exception_payload(["a.py", "b.py", "c.py"])
    payload["frames"][1]["function"] = "_internal_helper"

    result = filter_frames(payload, exclude_functions=["_internal"])

    assert_frame_functions(result, ["func_0", "func_2"], "exception frames")
    _assert_exception_identity(result, "ValueError", "test error", "filtered exception")


def test_exc_exclude_functions_in_cause() -> None:
    """Function patterns should exclude matching frames in cause chain."""
    cause = make_exception_payload(
        ["cause_a.py", "cause_b.py"],
        type_name="OSError",
        message="cause error",
    )
    cause["frames"][0]["function"] = "_internal_cause"
    payload = make_exception_payload(["main.py"])
    payload["cause"] = cause

    result = filter_frames(payload, exclude_functions=["_internal"])

    assert_frame_functions(result["cause"], ["func_1"], "cause frames")


def test_exc_filters_deep_cause_chain() -> None:
    """A deeply nested cause chain should be filtered at every level."""
    current = make_exception_payload(
        ["base.py"],
        type_name="BaseError",
        message="root cause",
    )
    for level in range(1, _DEEP_CHAIN_LENGTH):
        wrapper = make_exception_payload(
            [f"level_{level}.py", "femtologging/__init__.py"],
            type_name=f"Error{level}",
            message=f"level {level}",
        )
        wrapper["cause"] = current
        current = wrapper

    result = filter_frames(current, exclude_logging=True)

    # The outermost wrapper is the deepest level; the base exception is last.
    expected_filenames = [
        f"level_{level}.py" for level in range(_DEEP_CHAIN_LENGTH - 1, 0, -1)
    ]
    expected_filenames.append("base.py")

    node: FilteredPayload | None = result
    for depth, expected_filename in enumerate(expected_filenames):
        assert node is not None, (
            f"cause chain ended at depth {depth}, expected "
            f"{len(expected_filenames)} linked exceptions"
        )
        assert_frame_filenames(node, [expected_filename], f"cause chain depth {depth}")
        node = node.get("cause")

    assert node is None, (
        f"cause chain should end after the root cause, found extra link {node}"
    )
