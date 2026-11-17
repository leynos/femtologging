"""Skip here-document bodies while reading a workflow step's shell text.

A here-document hands the shell the lines that follow it verbatim, up to a
delimiter line. Those lines are data, not shell, so a quote inside one must
not be read as opening a quoted string. Without that, an apostrophe in a
here-document body would open a single quote that swallows every line below
it, including a later `cargo test` the provisioning contract has to see.

Reading the operator is deliberately approximate: it is found wherever it
appears in the joined command, even inside a quoted string. A false match only
costs the lines it skips when a matching delimiter line actually follows, and
that is rarer than the miss it prevents.
"""

from __future__ import annotations

import re
import typing as typ

#: A here-document operator and its delimiter. The delimiter may be bare or
#: single- or double-quoted, and `<<-` strips leading tabs from the body.
_HEREDOC: typ.Final[re.Pattern[str]] = re.compile(
    r"<<(-?)[ \t]*(?:'([^']*)'|\"([^\"]*)\"|([A-Za-z_][A-Za-z0-9_]*))"
)


class HereDocument(typ.NamedTuple):
    """One here-document a command opens."""

    delimiter: str
    strips_leading_tabs: bool


def here_documents(command: str) -> list[HereDocument]:
    """Return the here-documents `command` opens, in order.

    Parameters
    ----------
    command
        One joined shell command.

    Returns
    -------
    list[HereDocument]
        One entry per operator, in the order the shell reads them.
    """
    return [
        HereDocument(
            delimiter=next(part for part in match.groups()[1:] if part is not None),
            strips_leading_tabs=match.group(1) == "-",
        )
        for match in _HEREDOC.finditer(command)
    ]


def skip_here_documents(lines: list[str], start: int, command: str) -> int:
    """Return the index of the first line after `command`'s here-documents.

    Parameters
    ----------
    lines
        Every physical line of the step's ``run`` body.
    start
        Index of the first line a here-document body may occupy.
    command
        The joined command whose here-documents are being read.

    Returns
    -------
    int
        The index of the first line the shell reads as shell again.

    Examples
    --------
    >>> skip_here_documents(
    ...     ["cat <<'EOF'", "Don't", "EOF", "cargo test"], 0, "cat <<'EOF'"
    ... )
    3
    """
    index = start
    for document in here_documents(command):
        while index < len(lines):
            candidate = lines[index]
            index += 1
            if document.strips_leading_tabs:
                candidate = candidate.lstrip("\t")
            if candidate == document.delimiter:
                break
    return index
