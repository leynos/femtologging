"""Generated-input tests for the stack-frame stripping invariants.

``strip_external_stack_frames`` is documented to delete whole frame blocks and
to leave everything else byte-identical, including an exception traceback's
line endings. These Hypothesis properties hold that claim across arbitrary line
text and terminators, rather than only at the handwritten cases in
:mod:`tests.steps.test_external_stack_frames`.
"""

from __future__ import annotations

from pathlib import Path

from tests.steps._hypothesis_support import (
    _FRAME_PATH_PROPERTY_CASES,
    _LINE_PROPERTY_CASES,
)
from tests.steps.stack_frames import strip_external_stack_frames

_STDLIB_ROOT = Path("/opt/python/lib/python3.12")
_EXTERNAL_ROOTS = (_STDLIB_ROOT,)
_STACK_HEADING = "Stack (most recent call last):"
_STACK_LINES = (
    _STACK_HEADING,
    '  File "/opt/python/lib/python3.12/runpy.py", line 9, in _run',
    "    return _run_code()",
    '  File "app.py", line 7, in emit',
    "    logger.log(level, message)",
)


def _lines(text: str) -> list[str]:
    """Return the text's lines, each keeping its own terminator."""
    return text.splitlines(keepends=True)


def _is_line_subsequence(kept: list[str], original: list[str]) -> bool:
    """Return whether ``kept`` is ``original`` minus zero or more whole lines."""
    remaining = iter(original)
    return all(any(line == candidate for candidate in remaining) for line in kept)


def _build_output(
    bodies: list[str],
    ending: str,
    *,
    ends_with_terminator: bool,
) -> str:
    """Join generated line bodies with ``ending``, optionally ending without one."""
    output = ending.join(bodies)
    if bodies and ends_with_terminator:
        output += ending
    return output


class TestStrippingPreservesSurvivingLines:
    """Properties that hold wherever the frame blocks fall."""

    @staticmethod
    @_LINE_PROPERTY_CASES
    def test_stripping_removes_only_whole_lines(
        bodies: list[str],
        ending: str,
        *,
        ends_with_terminator: bool,
    ) -> None:
        """Delete whole lines and keep every surviving line's terminator."""
        output = _build_output(
            bodies, ending, ends_with_terminator=ends_with_terminator
        )
        stripped = strip_external_stack_frames(output, external_roots=_EXTERNAL_ROOTS)
        assert _is_line_subsequence(_lines(stripped), _lines(output)), (
            "stripping must only remove whole lines, never rewrite a kept one"
        )
        assert (
            strip_external_stack_frames(stripped, external_roots=_EXTERNAL_ROOTS)
            == stripped
        ), "strip_external_stack_frames must be idempotent"

    @staticmethod
    @_LINE_PROPERTY_CASES
    def test_a_traceback_tail_survives_byte_identically(
        bodies: list[str],
        ending: str,
        *,
        ends_with_terminator: bool,
    ) -> None:
        """Leave the exception section byte-identical, terminators included.

        Each generated line carries a ``> `` prefix so that none can be
        mistaken for a section heading, which would legitimately start a new
        section rather than being preserved as traceback text.
        """
        tail = (
            "Traceback (most recent call last):"
            + ending
            + "".join(f"> {body}{ending}" for body in bodies)
        )
        if not ends_with_terminator:
            tail = tail.removesuffix(ending)
        output = ending.join(_STACK_LINES) + ending + tail
        stripped = strip_external_stack_frames(output, external_roots=_EXTERNAL_ROOTS)
        assert stripped.endswith(tail), (
            "an exception traceback must survive stripping byte for byte"
        )


class TestFrameAttribution:
    """Properties of the rule that decides which frames are external."""

    @staticmethod
    @_FRAME_PATH_PROPERTY_CASES
    def test_frames_beneath_an_external_root_are_removed(segment: str) -> None:
        """Drop a frame beneath a supplied root and keep a relative frame."""
        external = f"{_STDLIB_ROOT}/{segment}/mod.py"
        application = f"relative/{segment}/app.py"
        output = (
            f"{_STACK_HEADING}\n"
            f'  File "{external}", line 3, in run\n'
            "    run_code()\n"
            f'  File "{application}", line 7, in emit\n'
            "    logger.log(level, message)\n"
        )
        stripped = strip_external_stack_frames(output, external_roots=_EXTERNAL_ROOTS)
        assert external not in stripped, (
            "a frame beneath an external root must be removed"
        )
        assert application in stripped, (
            "a relative frame must be kept because it cannot be attributed"
        )
