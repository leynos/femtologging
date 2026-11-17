"""Render captured exception payloads, including nested chains and groups.

The stdlib adapter stores this text in ``LogRecord.exc_text``, which the
stdlib :class:`logging.Formatter` prints without rebuilding the traceback. The
rendering lives apart from :mod:`femtologging._payload_formatting` so both
modules stay inside the project's file-size ceiling; the marker helpers they
share stay in that module.

Nested payloads are rendered through the same path as the root, so a chained
cause, an implicit context and an exception-group member each report their own
traceback, message and omission markers. Without that, a marker captured on a
nested payload would never reach the emitted text.
"""

from __future__ import annotations

import collections.abc as cabc

from ._payload_formatting import (
    _format_chain_omissions,
    _format_exception_summary,
    _format_frame_omission,
    _format_frames,
    _format_group_omissions,
)

_CAUSE_SEPARATOR = (
    "\nThe above exception was the direct cause of the following exception:\n"
)
_CONTEXT_SEPARATOR = (
    "\nDuring handling of the above exception, another exception occurred:\n"
)


def _truncation_marker(payload: cabc.Mapping[str, object]) -> cabc.Mapping[str, object]:
    """Return the payload's truncation marker, or an empty mapping.

    Returns
    -------
    collections.abc.Mapping
        The marker fields, or an empty mapping when the payload has none.

    Examples
    --------
    >>> _truncation_marker({"truncation": {"frames_omitted": 1}})
    {'frames_omitted': 1}
    >>> _truncation_marker({})
    {}

    """
    marker = payload.get("truncation")
    return marker if isinstance(marker, cabc.Mapping) else {}


def _nested_payload(value: object) -> cabc.Mapping[str, object] | None:
    """Return ``value`` when it is a nested exception payload.

    Returns
    -------
    collections.abc.Mapping or None
        The nested payload, or ``None`` for any other value.

    Examples
    --------
    >>> _nested_payload({"type_name": "ValueError"})
    {'type_name': 'ValueError'}
    >>> _nested_payload(None) is None
    True

    """
    return value if isinstance(value, cabc.Mapping) else None


def _retained_cause(
    exc_info: cabc.Mapping[str, object],
) -> cabc.Mapping[str, object] | None:
    """Return the cause payload the formatter displays, if any.

    Returns
    -------
    collections.abc.Mapping or None
        The cause payload, or ``None`` when absent or malformed.

    """
    return _nested_payload(exc_info.get("cause"))


def _retained_context(
    exc_info: cabc.Mapping[str, object],
) -> cabc.Mapping[str, object] | None:
    """Return the context payload the formatter displays, if any.

    A cause takes precedence over the context, and ``suppress_context`` hides
    the context outright, so a context is displayed only when neither applies.

    Returns
    -------
    collections.abc.Mapping or None
        The context payload, or ``None`` when it would not be displayed.

    """
    if _retained_cause(exc_info) is not None:
        return None
    if exc_info.get("suppress_context") is True:
        return None
    return _nested_payload(exc_info.get("context"))


def _format_exc_text(exc_info: cabc.Mapping[str, object]) -> str:
    """Render an exception payload, including any capture omissions.

    Chained causes, contexts and exception-group members are rendered through
    the same path, so a nested payload reports its own omitted frames, links
    and members rather than disappearing from the text.

    Returns
    -------
    str
        The traceback header, retained frames, exception message, and markers.

    Examples
    --------
    >>> print(_format_exc_text({"type_name": "ValueError", "message": "bad"}))
    Traceback (most recent call last):
    ValueError: bad

    """
    return "\n".join(_render_exception(exc_info))


def _render_exception(exc_info: cabc.Mapping[str, object]) -> list[str]:
    """Render one exception and everything chained beneath it.

    Returns
    -------
    list[str]
        Lines for the retained cause or context, this exception's own
        traceback, and any exception-group members.

    """
    lines = _render_chained_link(exc_info)
    lines.extend(_render_exception_body(exc_info))
    lines.extend(_render_exception_group(exc_info))
    return lines


def _render_chained_link(exc_info: cabc.Mapping[str, object]) -> list[str]:
    """Render the retained cause, or the retained context when it applies.

    Returns
    -------
    list[str]
        The nested exception's lines followed by the chain separator, or an
        empty list when no link is rendered.

    """
    cause = _retained_cause(exc_info)
    if cause is not None:
        return [*_render_exception(cause), _CAUSE_SEPARATOR]
    context = _retained_context(exc_info)
    if context is not None:
        return [*_render_exception(context), _CONTEXT_SEPARATOR]
    return []


def _render_exception_body(exc_info: cabc.Mapping[str, object]) -> list[str]:
    """Render this exception's frame markers, frames, and message.

    Returns
    -------
    list[str]
        The traceback header, omission markers, retained frames, the message
        line, and the payload summary.

    """
    marker = _truncation_marker(exc_info)
    lines = ["Traceback (most recent call last):"]
    lines.extend(
        _format_chain_omissions(
            marker,
            cause_displayed=_retained_cause(exc_info) is not None,
            context_displayed=_retained_context(exc_info) is not None,
        )
    )
    frame_omission = _format_frame_omission(marker)
    if frame_omission is not None:
        lines.append(frame_omission)

    frames = exc_info.get("frames", [])
    if isinstance(frames, list):
        lines.extend(
            _format_frames([
                frame for frame in frames if isinstance(frame, cabc.Mapping)
            ])
        )

    type_name = exc_info.get("type_name", "Exception")
    message = exc_info.get("message", "")
    lines.append(f"{type_name}: {message}" if message else str(type_name))
    summary = _format_exception_summary(marker)
    if summary is not None:
        lines.append(summary)
    return lines


def _render_exception_group(exc_info: cabc.Mapping[str, object]) -> list[str]:
    """Render exception-group members, each with its own markers.

    Returns
    -------
    list[str]
        Indented member blocks followed by the omitted-member count, or an
        empty list when this exception is not a group.

    Examples
    --------
    >>> lines = _render_exception_group({"exceptions": [{"type_name": "ValueError"}]})
    >>> lines[0]
    '  +---- [1] '
    >>> lines[-1]
    '  |     ValueError'

    """
    marker = _truncation_marker(exc_info)
    members = exc_info.get("exceptions", [])
    nested = (
        [member for member in members if isinstance(member, cabc.Mapping)]
        if isinstance(members, list)
        else []
    )
    omissions = _format_group_omissions(marker)
    if not nested and not omissions:
        return []
    lines: list[str] = []
    for index, member in enumerate(nested, start=1):
        lines.append(f"  +---- [{index}] ")
        lines.extend(f"  |     {line}" for line in _render_exception(member))
    lines.extend(omissions)
    return lines
