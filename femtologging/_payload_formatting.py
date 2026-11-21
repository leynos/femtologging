"""Formatting helpers for structured exception and stack payloads."""

from __future__ import annotations

import collections.abc as cabc


def _format_frames(frames: list[cabc.Mapping[str, object]]) -> cabc.Iterator[str]:
    """Render frame mappings as human-readable traceback lines.

    Yields
    ------
    str
        A formatted frame line, followed by its source line when available.

    Examples
    --------
    >>> list(_format_frames([{"filename": "a.py", "lineno": 2,
    ...                      "function": "run"}]))
    ['  File "a.py", line 2, in run']

    """
    for frame in frames:
        filename = frame.get("filename", "<unknown>")
        lineno = frame.get("lineno", 0)
        function = frame.get("function", "<unknown>")
        yield f'  File "{filename}", line {lineno}, in {function}'
        source = frame.get("source_line")
        if isinstance(source, str) and source:
            yield f"    {source}"


def _positive_count(marker: cabc.Mapping[str, object], field: str) -> int:
    """Return a positive integer count, ignoring booleans and invalid values.

    Returns
    -------
    int
        The positive count stored under ``field``, or zero.

    Examples
    --------
    >>> _positive_count({"frames_omitted": 3}, "frames_omitted")
    3
    >>> _positive_count({"frames_omitted": True}, "frames_omitted")
    0

    """
    value = marker.get(field, 0)
    return (
        value
        if isinstance(value, int) and not isinstance(value, bool) and value > 0
        else 0
    )


def _format_frame_omission(truncation: cabc.Mapping[str, object]) -> str | None:
    """Format an earlier-frame omission marker when the count is positive.

    Returns
    -------
    str or None
        The marker line, or ``None`` when no frames were omitted.

    Examples
    --------
    >>> _format_frame_omission({"frames_omitted": 2})
    '... 2 earlier frames omitted ...'
    >>> _format_frame_omission(
    ...     {"frames_omitted": 2, "frames_omitted_is_lower_bound": True}
    ... )
    '... at least 2 frames omitted ...'

    """
    omitted = _positive_count(truncation, "frames_omitted")
    if not omitted:
        return None
    if truncation.get("frames_omitted_is_lower_bound"):
        return f"... at least {omitted} frames omitted ..."
    return f"... {omitted} earlier frames omitted ..."


def _format_chain_omissions(
    truncation: cabc.Mapping[str, object],
    *,
    cause_displayed: bool = False,
    context_displayed: bool = False,
) -> list[str]:
    """Format marker lines for causes and contexts that were omitted.

    A link that was retained and displayed is not reported, because the marker
    explains absent text. Each link is decided on its own, so a retained cause
    does not hide an omitted context, or the reverse.

    Parameters
    ----------
    truncation
        Marker fields captured from an exception.
    cause_displayed
        Whether the retained cause was rendered in the text.
    context_displayed
        Whether the retained context was rendered in the text.

    Returns
    -------
    list[str]
        One line per omitted link, in cause-then-context order.

    Examples
    --------
    >>> _format_chain_omissions({"cause_omitted": True})
    ['... exception cause omitted ...']
    >>> _format_chain_omissions({"cause_omitted": True}, cause_displayed=True)
    []

    """
    lines: list[str] = []
    if truncation.get("cause_omitted") is True and not cause_displayed:
        lines.append("... exception cause omitted ...")
    if truncation.get("context_omitted") is True and not context_displayed:
        lines.append("... exception context omitted ...")
    return lines


def _format_group_omissions(truncation: cabc.Mapping[str, object]) -> list[str]:
    """Format a marker line for exception-group members omitted by count.

    Returns
    -------
    list[str]
        A one-line list when members were omitted, or an empty list.

    Examples
    --------
    >>> _format_group_omissions({"exceptions_omitted": 3})
    ['  ... and 3 more exceptions ...']

    """
    omitted = _positive_count(truncation, "exceptions_omitted")
    if omitted:
        return [f"  ... and {omitted} more exceptions ..."]
    return []


def _append_flag_detail(
    details: list[str], truncation: cabc.Mapping[str, object], field: str, text: str
) -> None:
    """Append ``text`` when the marker's ``field`` flag is true.

    Parameters
    ----------
    details
        Destination list for the summary details.
    truncation
        Marker fields captured from an exception.
    field
        Boolean marker name to inspect.
    text
        Summary detail to append when the field is true.

    Examples
    --------
    >>> details = []
    >>> _append_flag_detail(details, {"message_truncated": True},
    ...                    "message_truncated", "message truncated")
    >>> details
    ['message truncated']

    """
    if truncation.get(field) is True:
        details.append(text)


def _append_count_detail(
    details: list[str],
    truncation: cabc.Mapping[str, object],
    field: str,
    template: str,
) -> None:
    """Append a formatted detail when ``field`` contains a positive count.

    Parameters
    ----------
    details
        Destination list for the summary details.
    truncation
        Marker fields captured from an exception.
    field
        Count marker name to inspect.
    template
        Format string with one placeholder for the count.

    Examples
    --------
    >>> details = []
    >>> _append_count_detail(details, {"args_omitted": 2},
    ...                     "args_omitted", "{} arguments omitted")
    >>> details
    ['2 arguments omitted']

    """
    count = _positive_count(truncation, field)
    if count:
        details.append(template.format(count))


def _format_exception_summary(truncation: cabc.Mapping[str, object]) -> str | None:
    """Build the compact summary for exception fields that were truncated.

    Returns
    -------
    str or None
        A bounded summary of non-default marker fields, or ``None``.

    Examples
    --------
    >>> _format_exception_summary({"args_omitted": 2})
    '... exception payload truncated (2 arguments omitted) ...'

    """
    details: list[str] = []
    _append_flag_detail(details, truncation, "message_truncated", "message truncated")
    _append_count_detail(details, truncation, "args_omitted", "{} arguments omitted")
    _append_flag_detail(
        details, truncation, "args_truncated", "argument representations truncated"
    )
    _append_count_detail(details, truncation, "notes_omitted", "{} notes omitted")
    _append_flag_detail(details, truncation, "notes_truncated", "notes truncated")
    _append_count_detail(
        details,
        truncation,
        "failed_elements",
        "{} elements omitted after conversion failures",
    )
    _append_flag_detail(
        details, truncation, "members_truncated", "exception group members truncated"
    )
    _append_flag_detail(
        details, truncation, "budget_exhausted", "capture budget exhausted"
    )

    if not details:
        return None
    return f"... exception payload truncated ({'; '.join(details)}) ..."


def _format_stack_text(stack_info: cabc.Mapping[str, object]) -> str:
    """Render stack frames and any omitted-frame marker.

    Returns
    -------
    str
        The stack header, retained frames, and any frame omission marker.

    Examples
    --------
    >>> _format_stack_text({"frames": []})
    'Stack (most recent call last):'

    """
    lines = ["Stack (most recent call last):"]
    truncation = stack_info.get("truncation")
    marker = truncation if isinstance(truncation, cabc.Mapping) else {}
    frame_omission = _format_frame_omission(marker)
    if frame_omission is not None:
        lines.append(frame_omission)

    frames = stack_info.get("frames", [])
    if isinstance(frames, list):
        lines.extend(
            _format_frames([
                frame for frame in frames if isinstance(frame, cabc.Mapping)
            ])
        )
    return "\n".join(lines)
