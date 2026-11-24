r"""Expand ``%(name)s`` placeholders in INI handler parameters.

The scanner is a single forward pass rather than a backtracking regular
expression. An unanchored ``%\\(([^)]+)\\)s`` search restarts at every ``%(``
because its name group also accepts nested openers, so text holding many
unterminated openers rescans overlapping suffixes and costs quadratic time.
This module keeps the same leftmost, non-overlapping substitution semantics
while bounding the work to one pass over the input.
"""

from __future__ import annotations

import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

_OPEN_MARKER = "%("
_CLOSE_MARKER = ")"
_PLACEHOLDER_SUFFIX = "s"


def _iter_placeholders(
    value: str,
    find: cabc.Callable[[str, int], int] | None = None,
) -> cabc.Iterator[tuple[int, int, str]]:
    """Yield ``(start, end, name)`` for each ``%(name)s`` placeholder.

    Invariant: the cursor only ever moves forward. Each candidate pairs the
    earliest ``%(`` at or after the cursor with the first ``)`` after it, and a
    rejected candidate resumes just after its own closer, so no suffix is
    rescanned and malformed input costs linear time.

    Parameters
    ----------
    value
        Text to scan.
    find
        Optional ``(substring, start)`` search seam for tests; production
        callers must leave it unset.

    Yields
    ------
    tuple[int, int, str]
        Start offset, exclusive end offset, and name, in left-to-right order.

    Examples
    --------
    >>> list(_iter_placeholders("a=%(x)s b=%(y)s"))
    [(2, 7, 'x'), (10, 15, 'y')]
    >>> list(_iter_placeholders("%(unterminated")), list(_iter_placeholders("%()s"))
    ([], [])

    """
    search = value.find if find is None else find
    cursor = 0
    while True:
        start = search(_OPEN_MARKER, cursor)
        if start < 0:
            return
        closer = search(_CLOSE_MARKER, start + len(_OPEN_MARKER))
        if closer < 0:
            return
        name = value[start + len(_OPEN_MARKER) : closer]
        if not name or not value.startswith(_PLACEHOLDER_SUFFIX, closer + 1):
            cursor = closer + 1
            continue
        end = closer + len(_CLOSE_MARKER) + len(_PLACEHOLDER_SUFFIX)
        yield start, end, name
        cursor = end


def _expand_placeholders(value: str, defaults: cabc.Mapping[str, str]) -> str:
    """Expand ``%(name)s`` placeholders, rejecting names absent from *defaults*.

    Expansion is one pass over *value*: replacements are appended literally
    and never rescanned, so a default that itself contains a placeholder
    yields it verbatim, and ``%%`` has no special meaning.

    Returns
    -------
    str
        *value* with every recognised placeholder replaced.

    Raises
    ------
    ValueError
        If a placeholder names a key absent from *defaults*. The message
        quotes the original *value*, not a partially expanded form.

    Examples
    --------
    >>> _expand_placeholders("%(dir)s/app.log", {"dir": "/var/log"})
    '/var/log/app.log'
    >>> _expand_placeholders("%(dir)s", {"dir": "%(name)s"})
    '%(name)s'

    """
    if _OPEN_MARKER not in value:
        return value
    parts: list[str] = []
    cursor = 0
    for start, end, key in _iter_placeholders(value):
        if key not in defaults:
            msg = f"unknown placeholder {key!r} in {value!r}"
            raise ValueError(msg)
        parts.extend((value[cursor:start], defaults[key]))
        cursor = end
    parts.append(value[cursor:])
    return "".join(parts)
