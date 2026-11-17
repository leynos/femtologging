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
                }
            ],
        }
    })

    lines = exc_text.splitlines()
    member_heading = "  +---- [1] "
    member_marker = "  |     ... 2 earlier frames omitted ..."
    assert member_heading in lines, f"member block missing from {exc_text!r}"
    assert member_marker in lines, (
        f"a nested member's frame omission should reach the emitted text: {exc_text!r}"
    )
    assert lines.index(member_heading) < lines.index(member_marker), (
        "the member marker should sit inside the member's block"
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
