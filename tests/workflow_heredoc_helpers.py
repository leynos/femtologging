"""Read a here-document operator and skip the body it hands to the shell.

A here-document hands the shell the lines that follow it verbatim, up to a
delimiter line. Those lines are data, not shell, so a quote inside one must
not be read as opening a quoted string. Without that, an apostrophe in a
here-document body would open a single quote that swallows every line below
it, including a later `cargo test` the provisioning contract has to see.

Only an operator the shell reads opens a here-document: a `<<` inside a quoted
word is part of that word. This module therefore reads an operator at a given
offset rather than searching for one, and its caller supplies the offsets that
its own quoting scan found unquoted. One approximation remains: the scan does
not look inside a command substitution that is itself quoted, so a
here-document opened there is not seen.
"""

from __future__ import annotations

import re
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

#: A here-document operator and its delimiter. The delimiter may be bare or
#: single- or double-quoted, and `<<-` strips leading tabs from the body.
_OPERATOR: typ.Final[re.Pattern[str]] = re.compile(
    r"<<(-?)[ \t]*(?:'([^']*)'|\"([^\"]*)\"|([A-Za-z_][A-Za-z0-9_]*))"
)


class HereDocument(typ.NamedTuple):
    """One here-document a command opens."""

    delimiter: str
    strips_leading_tabs: bool


def here_document_at(command: str, at: int) -> HereDocument | None:
    """Return the here-document whose operator starts at `at`, if any.

    Parameters
    ----------
    command
        One joined shell command.
    at
        Offset of a `<<` the shell would read as an operator.

    Returns
    -------
    HereDocument | None
        The here-document the operator opens, or ``None`` when no delimiter
        follows. A delimiter shaped like a plain word is required, so the
        `<<` of an arithmetic shift opens nothing.

    Examples
    --------
    >>> here_document_at("cat <<'EOF'", 4)
    HereDocument(delimiter='EOF', strips_leading_tabs=False)
    >>> here_document_at("(( n << 2 ))", 6) is None
    True
    """
    match = _OPERATOR.match(command, at)
    if match is None:
        return None
    return HereDocument(
        delimiter=next(part for part in match.groups()[1:] if part is not None),
        strips_leading_tabs=match.group(1) == "-",
    )


def skip_here_documents(
    lines: list[str], start: int, documents: cabc.Sequence[HereDocument]
) -> int:
    """Return the index of the first line after `documents`' bodies.

    A here-document whose delimiter never appears is malformed shell, and the
    shell takes every remaining line as data. The same skip applies here.

    Parameters
    ----------
    lines
        Every physical line of the step's ``run`` body.
    start
        Index of the first line a here-document body may occupy.
    documents
        The here-documents the command opens, in the order the shell reads
        them.

    Returns
    -------
    int
        The index of the first line the shell reads as shell again.

    Examples
    --------
    >>> skip_here_documents(
    ...     ["cat <<'EOF'", "Don't", "EOF", "cargo test"],
    ...     0,
    ...     [HereDocument(delimiter="EOF", strips_leading_tabs=False)],
    ... )
    3
    """
    index = start
    for document in documents:
        while index < len(lines):
            candidate = lines[index]
            index += 1
            if document.strips_leading_tabs:
                candidate = candidate.lstrip("\t")
            if candidate == document.delimiter:
                break
    return index
