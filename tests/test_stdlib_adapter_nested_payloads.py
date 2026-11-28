"""Nested exception payloads must reach the stdlib adapter's ``exc_text``.

The stdlib :class:`logging.Formatter` prints ``LogRecord.exc_text`` verbatim
instead of rebuilding the traceback, so a marker the renderer forgets to emit
is lost to the reader. These tests observe the text the adapter stores when the
captured payload carries a chained or grouped exception.
"""

from __future__ import annotations

import typing as typ

from femtologging import StdlibHandlerAdapter
from tests.stdlib_adapter_support import CapturingHandler

if typ.TYPE_CHECKING:
    from femtologging.adapter import FemtoRecord


def _exc_text(record: FemtoRecord) -> str:
    r"""Return the exception text the adapter stores for *record*.

    Returns
    -------
    str
        The rendered exception text.

    Examples
    --------
    >>> _exc_text({"exc_info": {"type_name": "ValueError", "message": "bad"}})
    'Traceback (most recent call last):\nValueError: bad'

    """
    capturing = CapturingHandler()
    StdlibHandlerAdapter(capturing).handle_record(record)
    assert len(capturing.records) == 1, (
        f"expected 1 emitted record, got {len(capturing.records)}"
    )
    exc_text = capturing.records[0].exc_text
    assert isinstance(exc_text, str), f"exc_text should be rendered, got {exc_text!r}"
    return exc_text


def test_group_member_frame_omission_reaches_exc_text() -> None:
    """A group member's own frame marker should appear inside its block."""
    exc_text = _exc_text({
        "exc_info": {
            "schema_version": 1,
            "type_name": "ExceptionGroup",
            "message": "several",
            "frames": [],
            "exceptions": [
                {
                    "schema_version": 1,
                    "type_name": "ValueError",
                    "message": "bad",
                    "frames": [],
                    "truncation": {"frames_omitted": 2},
                },
                {
                    "schema_version": 1,
                    "type_name": "TypeError",
                    "message": "intact",
                    "frames": [],
                },
            ],
        }
    })

    lines = exc_text.splitlines()
    member_heading = "  +---- [1] "
    second_heading = "  +---- [2] "
    member_marker = "  |     ... 2 earlier frames omitted ..."
    assert member_heading in lines, f"member block missing from {exc_text!r}"
    assert second_heading in lines, f"second member missing from {exc_text!r}"
    assert member_marker in lines, (
        f"a nested member's frame omission should reach the emitted text: {exc_text!r}"
    )
    # Bounding the marker between the two headings shows it belongs to the
    # first member: a marker emitted after the whole group would still follow
    # the first heading but would land past the second.
    assert (
        lines.index(member_heading)
        < lines.index(member_marker)
        < lines.index(second_heading)
    ), "the member marker should sit inside the affected member's block"


def test_multiline_group_member_stays_inside_its_block() -> None:
    """Every physical line of a member message should carry the block indent."""
    exc_text = _exc_text({
        "exc_info": {
            "schema_version": 1,
            "type_name": "ExceptionGroup",
            "message": "several",
            "frames": [],
            "exceptions": [
                {
                    "schema_version": 1,
                    "type_name": "ValueError",
                    "message": "first\nsecond",
                    "frames": [],
                }
            ],
        }
    })

    lines = exc_text.splitlines()
    assert "  |     ValueError: first" in lines, (
        f"the member heading line is missing from {exc_text!r}"
    )
    assert "  |     second" in lines, (
        f"every physical line of a member's message should stay inside its "
        f"block: {exc_text!r}"
    )


def test_displayed_cause_is_not_reported_as_omitted() -> None:
    """A retained cause should render without claiming the cause was omitted."""
    exc_text = _exc_text({
        "exc_info": {
            "schema_version": 1,
            "type_name": "RuntimeError",
            "message": "wrapper",
            "frames": [],
            "cause": {
                "schema_version": 1,
                "type_name": "OSError",
                "message": "root cause",
                "frames": [],
            },
            "truncation": {"cause_omitted": True},
        }
    })

    assert "OSError: root cause" in exc_text, (
        f"the retained cause should be rendered: {exc_text!r}"
    )
    assert "... exception cause omitted ..." not in exc_text, (
        f"a displayed cause must not also be reported as omitted: {exc_text!r}"
    )


def _stack_lines(record: FemtoRecord) -> list[str]:
    """Return the stack text the adapter stores for *record*."""
    capturing = CapturingHandler()
    StdlibHandlerAdapter(capturing).handle_record(record)
    assert len(capturing.records) == 1, (
        f"expected 1 emitted record, got {len(capturing.records)}"
    )
    stack_info = capturing.records[0].stack_info
    assert isinstance(stack_info, str), (
        f"stack_info should be rendered, got {stack_info!r}"
    )
    return stack_info.splitlines()


def test_a_stack_budget_exhaustion_is_rendered() -> None:
    """A stack cut without dropping a frame should still explain the loss."""
    lines = _stack_lines({
        "stack_info": {
            "frames": [{"filename": "app.py", "lineno": 8, "function": "run"}],
            "truncation": {"budget_exhausted": True},
        }
    })

    assert "... exception payload truncated (capture budget exhausted) ..." in lines, (
        "a stack summary must report aggregate-budget exhaustion"
    )
