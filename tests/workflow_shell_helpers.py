"""Scan the shell quoting that decides how a workflow ``run`` body is read.

A ``run`` body is shell text, so its quotes, escapes and comments decide both
where one command ends and where a here-document operator is real rather than
part of a word. The line reader and the here-document reader need that same
decision, so the scanning primitives live here and each caller adds only its
own question: one asks whether the line continues, the other where an
unquoted `<<` starts.
"""

from __future__ import annotations

import typing as typ

# A ``#`` opens a shell comment only at the start of a word, so it is literal
# inside a word such as ``a#b``. A word begins at the start of the line or
# just after one of these separators.
WORD_SEPARATORS: typ.Final[frozenset[str]] = frozenset(" \t;&|()<>")
QUOTE_CHARS: typ.Final[frozenset[str]] = frozenset("'\"")


class LineScan(typ.NamedTuple):
    """The shell state one line leaves behind and what the join must remove."""

    quote: str | None
    continues: bool
    drops_trailing_backslash: bool


class ScanStep(typ.NamedTuple):
    """Where scanning resumes, the quote state, and whether the line joins."""

    index: int
    quote: str | None
    drops_trailing_backslash: bool


def resume(
    index: int, quote: str | None, *, drops_trailing_backslash: bool = False
) -> ScanStep:
    """Return the step that resumes scanning at `index` inside `quote`."""
    return ScanStep(
        index=index,
        quote=quote,
        drops_trailing_backslash=drops_trailing_backslash,
    )


def starts_comment(line: str, index: int) -> bool:
    """Report whether the ``#`` at `index` opens a shell comment."""
    return index == 0 or line[index - 1] in WORD_SEPARATORS


def scan_single_quoted(char: str, index: int, quote: str | None) -> ScanStep:
    """Consume a character in a single-quoted string, where backslash is literal."""
    return resume(index + 1, None if char == "'" else quote)


def scan_double_quoted(line: str, index: int, quote: str | None) -> ScanStep:
    """Consume a character inside a double-quoted string, honouring escapes."""
    char = line[index]
    if char == "\\" and index + 1 == len(line):
        return resume(index + 1, quote, drops_trailing_backslash=True)
    if char == "\\":
        return resume(index + 2, quote)
    return resume(index + 1, None if char == '"' else quote)


def scan_unquoted(line: str, index: int) -> ScanStep:
    """Consume a character outside quotes, where quotes and comments begin."""
    char = line[index]
    if char == "\\" and index + 1 == len(line):
        return resume(index + 1, None, drops_trailing_backslash=True)
    if char == "\\":
        return resume(index + 2, None)
    if char in QUOTE_CHARS:
        return resume(index + 1, char)
    if char == "#" and starts_comment(line, index):
        return resume(len(line), None)
    return resume(index + 1, None)


def scan_step(line: str, index: int, quote: str | None) -> ScanStep:
    """Consume the character at `index` in the state left by `quote`."""
    if quote == "'":
        return scan_single_quoted(line[index], index, quote)
    if quote == '"':
        return scan_double_quoted(line, index, quote)
    return scan_unquoted(line, index)


def scan_line(line: str, quote: str | None) -> LineScan:
    r"""Scan one shell line, carrying `quote` in and reporting how it ends.

    A backslash joins the next line only when it is the line's last character,
    outside a comment and outside single quotes. Two backslashes are an
    escaped backslash, so the second is not a continuation. A quoted string
    may hold a newline, which keeps the line open until its closing quote
    appears.

    Parameters
    ----------
    line
        One physical line of a ``run`` body, without its newline.
    quote
        The quote character left open by the previous line, if any.

    Returns
    -------
    LineScan
        The open quote, whether the line continues, and whether the joining
        backslash must be removed.
    """
    index = 0
    while index < len(line):
        step = scan_step(line, index, quote)
        if step.drops_trailing_backslash:
            return LineScan(
                quote=step.quote,
                continues=True,
                drops_trailing_backslash=True,
            )
        index, quote = step.index, step.quote
    return LineScan(
        quote=quote,
        continues=quote is not None,
        drops_trailing_backslash=False,
    )
