"""Unit tests for exception payload filtering."""

from __future__ import annotations

from femtologging import filter_frames

from .conftest import make_exception_payload


def test_exc_detects_exception_payload() -> None:
    """Exception payloads should be detected by type_name and message."""
    payload = make_exception_payload(
        ["myapp/main.py", "femtologging/__init__.py"],
    )

    result = filter_frames(payload, exclude_logging=True)

    # Should preserve exception fields
    assert result["type_name"] == "ValueError", "type_name should be preserved"
    assert result["message"] == "test error", "message should be preserved"
    assert len(result["frames"]) == 1, "expected 1 frame after filtering"


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
    assert len(result["frames"]) == 1, "expected 1 main frame"
    assert result["frames"][0]["filename"] == "main.py", "main frame mismatch"

    # Cause frames also filtered
    assert "cause" in result, "cause should be present"
    assert len(result["cause"]["frames"]) == 1, "expected 1 cause frame"
    assert result["cause"]["frames"][0]["filename"] == "cause.py", (
        "cause frame mismatch"
    )


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

    assert "context" in result, "context should be present"
    assert len(result["context"]["frames"]) == 1, "expected 1 context frame"
    assert result["context"]["frames"][0]["filename"] == "context.py", (
        "context frame mismatch"
    )


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

    assert len(result["exceptions"]) == 2, "expected 2 exception group members"
    assert len(result["exceptions"][0]["frames"]) == 1, "expected 1 frame in exc1"
    assert result["exceptions"][0]["frames"][0]["filename"] == "exc1.py", (
        "exc1 frame mismatch"
    )
    assert len(result["exceptions"][1]["frames"]) == 1, "expected 1 frame in exc2"
    assert result["exceptions"][1]["frames"][0]["filename"] == "exc2.py", (
        "exc2 frame mismatch"
    )


def test_exc_preserves_exception_fields() -> None:
    """All exception fields should be preserved."""
    payload = make_exception_payload(["main.py"])
    payload["module"] = "myapp.errors"
    payload["args_repr"] = ["'key'"]
    payload["notes"] = ["check the input"]
    payload["suppress_context"] = True

    result = filter_frames(payload)

    assert result["type_name"] == "ValueError", "type_name should be preserved"
    assert result["message"] == "test error", "message should be preserved"
    assert result["module"] == "myapp.errors", "module should be preserved"
    assert result["args_repr"] == ["'key'"], "args_repr should be preserved"
    assert result["notes"] == ["check the input"], "notes should be preserved"
    assert result["suppress_context"] is True, "suppress_context should be preserved"


def test_exc_exclude_functions() -> None:
    """Function patterns should exclude matching frames in exception payloads."""
    payload = make_exception_payload(["a.py", "b.py", "c.py"])
    payload["frames"][1]["function"] = "_internal_helper"

    result = filter_frames(payload, exclude_functions=["_internal"])

    assert len(result["frames"]) == 2, "expected 2 frames after filtering"
    functions = [f["function"] for f in result["frames"]]
    assert "_internal_helper" not in functions, "internal function should be excluded"
    # Should preserve exception fields
    assert result["type_name"] == "ValueError", "type_name should be preserved"
    assert result["message"] == "test error", "message should be preserved"


def test_exc_exclude_functions_in_cause() -> None:
    """Function patterns should exclude matching frames in cause chain."""
    cause = make_exception_payload(
        ["cause_a.py", "cause_b.py"],
        type_name="IOError",
        message="cause error",
    )
    cause["frames"][0]["function"] = "_internal_cause"
    payload = make_exception_payload(["main.py"])
    payload["cause"] = cause

    result = filter_frames(payload, exclude_functions=["_internal"])

    # Cause frames should be filtered
    assert len(result["cause"]["frames"]) == 1, "expected 1 cause frame"
    assert result["cause"]["frames"][0]["function"] == "func_1", (
        "remaining cause function mismatch"
    )


def test_exc_filters_deep_cause_chain() -> None:
    """Deep cause chain (100 levels) should be recursively filtered."""
    # Build a 100-level nested cause chain
    current = make_exception_payload(
        ["base.py"],
        type_name="BaseError",
        message="root cause",
    )
    for i in range(1, 100):
        wrapper = make_exception_payload(
            [f"level_{i}.py", "femtologging/__init__.py"],
            type_name=f"Error{i}",
            message=f"level {i}",
        )
        wrapper["cause"] = current
        current = wrapper

    result = filter_frames(current, exclude_logging=True)

    # Verify filtering was applied recursively
    # The outermost should have 1 frame (femtologging filtered)
    assert len(result["frames"]) == 1, "expected 1 frame at top level"

    # Walk the chain and verify each level was filtered
    depth = 0
    node = result
    while "cause" in node and node["cause"] is not None:
        depth += 1
        node = node["cause"]
        # Each level should have 1 frame after filtering
        assert len(node["frames"]) == 1, f"expected 1 frame at depth {depth}"

    # Should have traversed 99 cause links (100 total exceptions)
    assert depth == 99, f"expected 99 cause links, got {depth}"
