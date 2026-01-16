"""Shared fixtures and types for frame filter tests."""

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
