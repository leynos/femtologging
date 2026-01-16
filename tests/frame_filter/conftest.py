"""Shared fixtures and types for frame filter tests.

This module provides TypedDict definitions and factory functions for building
test payloads used across all frame filter tests.

Types exported:
    - FrameDict: TypedDict for stack frame structure
    - StackPayload: TypedDict for stack_info payloads
    - ExceptionPayload: TypedDict for exc_info payloads (extends StackPayload)

Factory functions:
    - make_frame(filename, lineno, function) -> FrameDict
    - make_stack_payload(filenames) -> StackPayload
    - make_exception_payload(filenames, type_name, message) -> ExceptionPayload

Example usage::

    from tests.frame_filter.conftest import make_stack_payload, StackPayload
    from femtologging import filter_frames

    def test_filters_venv_frames() -> None:
        payload = make_stack_payload(["app.py", ".venv/lib/foo.py"])
        result = filter_frames(payload, exclude_filenames=[".venv/"])
        assert len(result["frames"]) == 1
"""

from __future__ import annotations

import typing as typ


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


def make_frame(filename: str, lineno: int, function: str) -> FrameDict:
    """Create a frame dict."""
    return FrameDict(filename=filename, lineno=lineno, function=function)


def make_stack_payload(filenames: list[str]) -> StackPayload:
    """Create a stack_info payload dict with the given filenames."""
    frames: list[FrameDict] = [
        make_frame(fn, i + 1, f"func_{i}") for i, fn in enumerate(filenames)
    ]
    return StackPayload(schema_version=1, frames=frames)


def make_exception_payload(
    filenames: list[str],
    type_name: str = "ValueError",
    message: str = "test error",
) -> ExceptionPayload:
    """Create an exc_info payload dict with the given filenames."""
    frames: list[FrameDict] = [
        make_frame(fn, i + 1, f"func_{i}") for i, fn in enumerate(filenames)
    ]
    return ExceptionPayload(
        schema_version=1,
        frames=frames,
        type_name=type_name,
        message=message,
    )
