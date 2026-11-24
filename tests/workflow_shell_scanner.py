"""A shell scanner for a workflow step's ``run`` script.

A workflow step's script is a small shell program, and the contract that holds
the heavy lane's matrix in place has to ask two questions of it: is the driver
the whole command, and does any command turn exit-on-error off. Answering
either needs a reader that understands quoting.

``shlex`` is not that reader here. It discards whether a token was quoted, so
``echo ';' set +e`` and ``echo ready; set +e`` tokenize alike, and the first
has no ``set`` command at all. This scanner keeps that distinction: an
unquoted ``;`` separates commands, a quoted one is an argument.

Two views are offered because the callers need different ones.
:func:`tokenize` keeps a line whole, so ``uv run x || true`` is one entry and a
suppression suffix stays visible. :func:`tokenize_statements` splits a line
into its simple commands, so a ``set +e`` written after another command is
still found.
"""

from __future__ import annotations

import dataclasses as dc
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

#: Tokens that separate one shell command from the next, when unquoted.
SEPARATORS: typ.Final[frozenset[str]] = frozenset({"&&", "||", ";", "|", "&"})

#: Tokens that redirect a command's streams. The word after one names the
#: redirection's target, so it is skipped rather than read as an argument, and
#: scanning continues past it so a later command is still seen.
REDIRECTIONS: typ.Final[frozenset[str]] = frozenset({
    ">",
    ">>",
    ">&",
    "<",
    "<<",
    "<&",
})

#: Characters that begin an operator rather than a word.
_OPERATOR_CHARS: typ.Final[frozenset[str]] = frozenset(";&|<>()")

#: Operators, longest first so ``&&`` is preferred over ``&``.
_OPERATORS: typ.Final[tuple[str, ...]] = (
    "&&",
    "||",
    ";;",
    ">>",
    ">&",
    "<<",
    "<&",
    ";",
    "|",
    "&",
    ">",
    "<",
    "(",
    ")",
)

#: Quote characters the scanner treats as grouping rather than as content.
_QUOTES: typ.Final[frozenset[str]] = frozenset({'"', "'"})


@dc.dataclass(frozen=True, slots=True)
class Token:
    """One shell word or operator, with whether quoting produced it."""

    text: str
    is_quoted: bool = False


def _operator_at(line: str, index: int) -> tuple[str, int]:
    """Return the operator starting at ``index`` and its width.

    Returns
    -------
        The operator text and how many characters it spans.
    """
    for operator in _OPERATORS:
        if line.startswith(operator, index):
            return operator, len(operator)
    return line[index], 1


def _escapes_in(quote: str, char: str, line: str, index: int) -> bool:
    """Report whether ``char`` starts an escape that is in effect here.

    A backslash escapes the next character inside double quotes and outside
    quotes; inside single quotes it is literal.

    Returns
    -------
        True when the character at ``index`` is a backslash with something
        after it.
    """
    return quote == '"' and char == "\\" and index + 1 < len(line)


def _read_quoted(line: str, start: int) -> tuple[int, str]:
    """Return the text inside the quote at ``start`` and the index after it.

    An unterminated quote takes the rest of the line. Such a script is a shell
    syntax error that fails the step on its own, so the reader stays tolerant
    rather than raising on a line whose meaning is already lost.

    Returns
    -------
        The quoted text, unquoted, and the index of the first character after
        the closing quote.
    """
    quote = line[start]
    index = start + 1
    text: list[str] = []
    while index < len(line):
        char = line[index]
        if char == quote:
            return index + 1, "".join(text)
        if _escapes_in(quote, char, line, index):
            text.append(line[index + 1])
            index += 2
            continue
        text.append(char)
        index += 1
    return index, "".join(text)


def _read_word(line: str, start: int) -> tuple[Token, int]:
    """Return the word starting at ``start`` and the index after it.

    Quoting is remembered rather than discarded: a quoted ``;`` is an argument
    to its command, while an unquoted one separates two commands.

    Returns
    -------
        The word and the index of the first character after it.
    """
    text: list[str] = []
    is_quoted = False
    index = start
    while index < len(line):
        char = line[index]
        if char.isspace() or char in _OPERATOR_CHARS:
            break
        if char in _QUOTES:
            is_quoted = True
            index, quoted = _read_quoted(line, index)
            text.append(quoted)
            continue
        if char == "\\" and index + 1 < len(line):
            text.append(line[index + 1])
            is_quoted = True
            index += 2
            continue
        text.append(char)
        index += 1
    return Token("".join(text), is_quoted=is_quoted), index


def scan(line: str) -> list[Token]:
    """Return one line's words and operators, in order.

    ``#`` starts a comment only where a word could start, so a comment
    containing an apostrophe is not read as a quotation, while ``a#b`` stays
    one word and a quoted ``#`` stays argument content.

    Returns
    -------
        The line's tokens, with separators and redirections included.
    """
    tokens: list[Token] = []
    index = 0
    while index < len(line):
        char = line[index]
        if char.isspace():
            index += 1
            continue
        if char == "#":
            break
        if char in _OPERATOR_CHARS:
            operator, width = _operator_at(line, index)
            tokens.append(Token(operator))
            index += width
            continue
        word, index = _read_word(line, index)
        tokens.append(word)
    return tokens


def _scan_lines(script: str) -> list[list[Token]]:
    """Return one script's tokens, one entry per non-empty line.

    Returns
    -------
        The tokenized lines, in the order the script runs them.
    """
    return [tokens for line in script.splitlines() if (tokens := scan(line))]


def tokenize(script: str) -> list[list[str]]:
    """Return one script's tokens, one entry per line.

    Separators are kept as their own tokens, so a line written as
    ``uv run x || true`` is one entry ending in the suppression rather than
    two commands. That is what lets a contract refuse a suppression suffix;
    a reader that split on separators would see only ``uv run x`` and accept
    it.

    >>> tokenize("uv run x || true")
    [['uv', 'run', 'x', '||', 'true']]
    >>> tokenize("echo 'a; b'")
    [['echo', 'a; b']]

    Returns
    -------
        The tokenized lines, in the order the script runs them.
    """
    return [[token.text for token in line] for line in _scan_lines(script)]


def _split_statement(tokens: cabc.Sequence[Token]) -> list[list[str]]:
    """Return one line's tokens, split into its simple commands.

    An unquoted redirection and the word after it are skipped, and scanning
    continues, so ``echo ready > out; set +e`` still yields ``set +e``. A
    quoted operator is part of its word, so ``echo ';' set +e`` yields one
    command and ``set +e`` there is only an argument.

    Returns
    -------
        The line's simple commands, in order, with empty ones dropped.
    """
    statements: list[list[str]] = []
    current: list[str] = []
    pending_target = False
    for token in tokens:
        if pending_target:
            pending_target = False
            continue
        if token.is_quoted:
            current.append(token.text)
            continue
        if token.text in REDIRECTIONS:
            # A file descriptor may sit immediately before the operator, as in
            # `2>&1`; it belongs to the redirection, not to the arguments.
            if current and current[-1].isdigit():
                current.pop()
            pending_target = True
            continue
        if token.text in SEPARATORS:
            statements.append(current)
            current = []
            continue
        current.append(token.text)
    statements.append(current)
    return [statement for statement in statements if statement]


def tokenize_statements(script: str) -> list[list[str]]:
    """Return one script's tokens, one entry per simple command.

    A line may hold several commands separated by ``;``, ``&&``, ``||``,
    ``|`` or ``&``. Each is returned separately, because a reader that looked
    only at a line's first word would miss a ``set +e`` written after another
    command on the same line.

    >>> tokenize_statements("echo ready; set +e")
    [['echo', 'ready'], ['set', '+e']]
    >>> tokenize_statements("echo ';' set +e")
    [['echo', ';', 'set', '+e']]

    Returns
    -------
        The tokenized simple commands, in the order the script runs them.
    """
    return [
        statement
        for tokens in _scan_lines(script)
        for statement in _split_statement(tokens)
    ]
