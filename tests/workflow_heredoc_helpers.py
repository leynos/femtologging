"""Read here-document operators and skip the bodies they hand to the shell.

A here-document hands the shell the lines that follow it verbatim, up to a
delimiter line. Those lines are data, not shell, so a quote inside one must
not be read as opening a quoted string. Without that, an apostrophe in a
here-document body would open a single quote that swallows every line below
it, including a later `cargo test` the provisioning contract has to see.

Only an operator the shell reads opens a here-document: a `<<` inside a quoted
word is part of that word, so `echo '<<EOF'` opens nothing. Neither is a `<<`
inside an arithmetic expression, where it is a shift: `(( n << shift ))` names
an operand, not a delimiter. The operators are therefore found with the
quoting scan the line reader already uses, with arithmetic nesting tracked
alongside it, and the skip then consumes only real bodies.

An unquoted body is expanded, and expansion removes each backslash-newline
pair, so a delimiter split across such a pair still ends the document. The
skip joins those pairs before comparing an unquoted delimiter; a quoted
delimiter suppresses expansion, and its body stays literal.

One approximation remains: the scan does not look inside a command
substitution that is itself quoted, so a here-document opened there is not
seen.
"""

from __future__ import annotations

import re
import typing as typ

from tests.workflow_shell_helpers import scan_step

if typ.TYPE_CHECKING:
    import collections.abc as cabc

#: A here-document operator and its delimiter. The delimiter may be bare or
#: single- or double-quoted, and `<<-` strips leading tabs from the body. A
#: bare delimiter is a shell word, so it may contain hyphens: matching only the
#: prefix of `<<EOF-END` would look for the wrong terminator and read the rest
#: of the script as body.
_OPERATOR: typ.Final[re.Pattern[str]] = re.compile(
    r"<<(-?)[ \t]*(?:'([^']*)'|\"([^\"]*)\"|([A-Za-z_][A-Za-z0-9_-]*))"
)


class HereDocument(typ.NamedTuple):
    """One here-document a command opens."""

    delimiter: str
    strips_leading_tabs: bool
    delimiter_is_quoted: bool


def _operator_offsets(command: str) -> cabc.Iterator[int]:
    """Yield where an unquoted `<<` starts in `command`, in order.

    The shell reads a here-document operator only outside quotes, so a `<<`
    inside a quoted word is data. Nor is a `<<` inside an arithmetic
    expression an operator: `(( n << shift ))` shifts `n`, and treating that
    `<<` as a here-document would make the skip consume every line below it.
    Arithmetic nesting is therefore tracked as the scan runs. A comment ends
    the scan, since nothing after it is shell.

    Yields
    ------
    int
        The offset of each operator, in the order the shell reads them.
    """
    index = 0
    quote: str | None = None
    arithmetic_depth = 0
    while index < len(command):
        if quote is None:
            if command.startswith("((", index):
                arithmetic_depth += 1
                index += 2
                continue
            if arithmetic_depth and command.startswith("))", index):
                arithmetic_depth -= 1
                index += 2
                continue
            if not arithmetic_depth and command.startswith("<<", index):
                yield index
                index += 2
                continue
        step = scan_step(command, index, quote)
        index, quote = step.index, step.quote


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
        follows. A delimiter must be shaped like a plain shell word, so a
        shift by a literal such as `2` opens nothing here; a shift whose
        operand is an identifier is excluded by `_operator_offsets`, which
        never offers an operator from inside arithmetic. The result also
        records whether the delimiter was quoted, because the shell expands
        an unquoted body and leaves a quoted one literal.

    Examples
    --------
    >>> here_document_at("cat <<'EOF'", 4)
    HereDocument(delimiter='EOF', strips_leading_tabs=False, delimiter_is_quoted=True)
    >>> here_document_at("cat <<EOF", 4)
    HereDocument(delimiter='EOF', strips_leading_tabs=False, delimiter_is_quoted=False)
    >>> here_document_at("(( n << 2 ))", 6) is None
    True
    """
    match = _OPERATOR.match(command, at)
    if match is None:
        return None
    return HereDocument(
        delimiter=next(part for part in match.groups()[1:] if part is not None),
        strips_leading_tabs=match.group(1) == "-",
        delimiter_is_quoted=match.group(4) is None,
    )


def here_documents(command: str) -> list[HereDocument]:
    """Return the here-documents the shell would read in `command`.

    Parameters
    ----------
    command
        One joined shell command.

    Returns
    -------
    list[HereDocument]
        One entry per operator the shell reads, in order. An operator inside a
        quoted word contributes nothing, and neither does an arithmetic shift.

    Examples
    --------
    >>> [document.delimiter_is_quoted for document in here_documents("cat <<'EOF'")]
    [True]
    >>> here_documents("cat <<EOF-END; (( n << shift ))")[0].delimiter
    'EOF-END'
    >>> here_documents("(( n << shift ))")
    []
    """
    return [
        document
        for at in _operator_offsets(command)
        for document in (here_document_at(command, at),)
        if document is not None
    ]


def _ends_with_continuation(line: str) -> bool:
    r"""Report whether `line` ends in a backslash the shell reads as a join.

    Two backslashes spell a literal one, so only an odd run of trailing
    backslashes continues the line.

    Parameters
    ----------
    line
        One physical line of a here-document body, without its newline.

    Returns
    -------
    bool
        Whether the shell joins the next physical line onto this one.

    Examples
    --------
    >>> _ends_with_continuation("E\\")
    True
    >>> _ends_with_continuation("E\\\\")
    False
    """
    trailing = len(line) - len(line.rstrip("\\"))
    return trailing % 2 == 1


def _unquoted_body_line(
    lines: list[str], start: int, *, strip_tabs: bool
) -> tuple[str, int]:
    r"""Return the expanded body line at `start` and the index after it.

    An unquoted here-document body is expanded, and expansion removes each
    backslash-newline pair. The shell therefore sees ``E\`` followed by
    ``OF`` as the single line ``EOF``, which ends a document opened with
    `<<EOF`; comparing physical lines would miss that terminator and read the
    commands below as body.

    The join comes first and the `<<-` tab strip second. A tab that starts a
    continuation line therefore ends up inside the joined line, where it
    keeps ``E`` and ``OF`` apart: the line is not the delimiter. Stripping
    each physical line beforehand would join them into ``EOF`` and end the
    document early.

    Parameters
    ----------
    lines
        Every physical line of the step's ``run`` body.
    start
        Index of the first physical line of this body line.
    strip_tabs
        Whether `<<-` makes the shell strip leading tabs from the joined
        line.

    Returns
    -------
    tuple[str, int]
        The expanded text of the body line, and the index of the next
        physical line.
    """
    pieces: list[str] = []
    index = start
    while index < len(lines):
        piece = lines[index]
        index += 1
        continues = _ends_with_continuation(piece)
        pieces.append(piece[:-1] if continues else piece)
        if not continues:
            break
    joined = "".join(pieces)
    return (joined.lstrip("\t") if strip_tabs else joined), index


def _quoted_body_line(
    lines: list[str], index: int, *, strip_tabs: bool
) -> tuple[str, int]:
    """Return the literal body line at `index` and the index after it.

    A quoted delimiter suppresses expansion, so its body keeps every backslash
    exactly as written and no continuation joins two physical lines.

    Parameters
    ----------
    lines
        Every physical line of the step's ``run`` body.
    index
        Index of the physical line to read.
    strip_tabs
        Whether `<<-` makes the shell strip leading tabs from the line.

    Returns
    -------
    tuple[str, int]
        The literal body line, and the index of the next physical line.
    """
    line = lines[index]
    return (line.lstrip("\t") if strip_tabs else line), index + 1


def skip_here_documents(lines: list[str], start: int, command: str) -> int:
    r"""Return the index of the first line after `command`'s here-documents.

    A here-document whose delimiter never appears is malformed shell, and the
    shell takes every remaining line as data. The same skip applies here.

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
    >>> skip_here_documents(
    ...     ["cat <<EOF", "payload", "E\\", "OF", "cargo test"], 0, "cat <<EOF"
    ... )
    4
    """
    index = start
    for document in here_documents(command):
        read_line = (
            _quoted_body_line if document.delimiter_is_quoted else _unquoted_body_line
        )
        while index < len(lines):
            line, index = read_line(
                lines, index, strip_tabs=document.strips_leading_tabs
            )
            if line == document.delimiter:
                break
    return index
