"""Property tests for the shell-grammar readers behind the workflow contract.

The contract's readers decide where one shell command ends and where a
here-document operator is real. Each property states the shell rule on its own
terms and then checks the reader against it, so the reader cannot vouch for
itself: an arithmetic command is generated from a small grammar and must yield
no here-document, a body is generated around a delimiter and the skip must
land on the line after it, and a delimiter is split across a continuation so
that only an unquoted body may join the halves back together.
"""

from __future__ import annotations

import typing as typ

from hypothesis import assume, given, settings
from hypothesis import strategies as st

from tests.workflow_command_helpers import _logical_lines
from tests.workflow_heredoc_helpers import here_documents, skip_here_documents
from tests.workflow_shell_helpers import LineScan, scan_line

DELIMITERS: typ.Final[tuple[str, ...]] = ("EOF", "END", "E-1")
#: Body lines the shell reads as data. The apostrophe is the motivating case:
#: read as shell it would open a string that swallows the lines below.
BODY_LINES: typ.Final[tuple[str, ...]] = (
    "payload",
    "Don't alter this text",
    "  spaced  ",
    "a#b",
    "not <<a delimiter",
)
TAIL_COMMANDS: typ.Final[tuple[str, ...]] = ("make test", "cargo test --locked")
ARITHMETIC_OPERANDS: typ.Final[tuple[str, ...]] = ("n", "count", "1", "42")
ARITHMETIC_OPERATORS: typ.Final[tuple[str, ...]] = ("+", "-", "*", "<<", ">>", "&", "|")
SPLIT_HEADS: typ.Final[tuple[str, ...]] = ("E", "EN", "AB")
SPLIT_TAILS: typ.Final[tuple[str, ...]] = ("OF", "D", "CD")
#: Words for quoting properties. None holds a quote character, so the quote
#: state of a generated line is decided by its opener alone.
QUOTED_WORDS: typ.Final[tuple[str, ...]] = ("alpha", "b#c", "<<EOF", "a-b", "x=y")


def _arithmetic_expression(draw: st.DrawFn, depth: int) -> str:
    """Return an arithmetic expression with at most `depth` nested groups.

    Returns
    -------
    str
        An expression built from operands, binary operators (shifts included)
        and parenthesised groups.
    """
    if depth == 0:
        return draw(st.sampled_from(ARITHMETIC_OPERANDS))
    shape = draw(st.sampled_from(("operand", "group", "binary")))
    if shape == "operand":
        return draw(st.sampled_from(ARITHMETIC_OPERANDS))
    if shape == "group":
        return f"({_arithmetic_expression(draw, depth - 1)})"
    operator = draw(st.sampled_from(ARITHMETIC_OPERATORS))
    left = _arithmetic_expression(draw, depth - 1)
    right = _arithmetic_expression(draw, depth - 1)
    return f"{left} {operator} {right}"


@st.composite
def _arithmetic_command(draw: st.DrawFn) -> str:
    """Generate an arithmetic command, where every `<<` shifts a value."""
    depth = draw(st.integers(min_value=0, max_value=2))
    return f"(( {_arithmetic_expression(draw, depth)} ))"


@settings(max_examples=50, deadline=None)
@given(command=_arithmetic_command())
def test_an_arithmetic_shift_never_opens_a_here_document(command: str) -> None:
    """Inside `(( … ))` a `<<` shifts a value, however the groups nest."""
    assert here_documents(command) == [], command


@settings(max_examples=50, deadline=None)
@given(
    delimiter=st.sampled_from(DELIMITERS),
    quote=st.sampled_from(("'", '"')),
    prefix=st.sampled_from(("echo", "printf %s", "true && echo")),
)
def test_a_quoted_operator_is_part_of_its_word(
    delimiter: str, quote: str, prefix: str
) -> None:
    """A `<<` inside a quoted word names no delimiter."""
    command = f"{prefix} {quote}before <<{delimiter} after{quote}"
    assert here_documents(command) == [], command


@settings(max_examples=50, deadline=None)
@given(
    delimiter=st.sampled_from(DELIMITERS),
    body=st.lists(st.sampled_from(BODY_LINES), max_size=5),
    tail=st.sampled_from(TAIL_COMMANDS),
)
def test_the_skip_lands_on_the_line_after_the_terminator(
    delimiter: str, body: list[str], tail: str
) -> None:
    """Every body line is data; the command after the terminator is shell."""
    command = f"cat <<{delimiter}"
    lines = [command, *body, delimiter, tail]
    assert skip_here_documents(lines, 1, command) == len(lines) - 1, lines
    assert _logical_lines("\n".join(lines)) == [command, tail], lines


@settings(max_examples=50, deadline=None)
@given(
    delimiter=st.sampled_from(DELIMITERS),
    body=st.lists(st.sampled_from(BODY_LINES), max_size=5),
    tail=st.sampled_from(TAIL_COMMANDS),
)
def test_a_missing_terminator_takes_every_remaining_line(
    delimiter: str, body: list[str], tail: str
) -> None:
    """A malformed here-document swallows the rest of the body, as the shell does."""
    command = f"cat <<{delimiter}"
    lines = [command, *body, tail]
    assert skip_here_documents(lines, 1, command) == len(lines), lines
    assert _logical_lines("\n".join(lines)) == [command], lines


@settings(max_examples=50, deadline=None)
@given(
    head=st.sampled_from(SPLIT_HEADS),
    tail=st.sampled_from(SPLIT_TAILS),
    after=st.sampled_from(TAIL_COMMANDS),
)
def test_an_unquoted_body_expands_before_comparing_the_delimiter(
    head: str, tail: str, after: str
) -> None:
    """A backslash-newline pair joins, so a split delimiter still ends the body."""
    delimiter = head + tail
    command = f"cat <<{delimiter}"
    lines = [command, "payload", f"{head}\\", tail, after]
    assert skip_here_documents(lines, 1, command) == len(lines) - 1, lines
    assert _logical_lines("\n".join(lines)) == [command, after], lines


@settings(max_examples=50, deadline=None)
@given(
    head=st.sampled_from(SPLIT_HEADS),
    tail=st.sampled_from(SPLIT_TAILS),
    after=st.sampled_from(TAIL_COMMANDS),
)
def test_a_quoted_body_keeps_the_backslash_and_its_lines_apart(
    head: str, tail: str, after: str
) -> None:
    """A quoted delimiter suppresses expansion, so only its own line ends it."""
    delimiter = head + tail
    command = f"cat <<'{delimiter}'"
    lines = [command, "payload", f"{head}\\", tail, delimiter, after]
    assert skip_here_documents(lines, 1, command) == len(lines) - 1, lines
    assert _logical_lines("\n".join(lines)) == [command, after], lines


@settings(max_examples=50, deadline=None)
@given(
    words=st.lists(st.sampled_from(QUOTED_WORDS), max_size=3),
    opener=st.sampled_from(("'", '"')),
)
def test_scan_line_keeps_an_unclosed_quote_open(words: list[str], opener: str) -> None:
    """A quote left open continues the line, whatever the line holds."""
    line = f"echo {opener}{' '.join(words)}"
    expected = LineScan(
        quote=opener,
        continues=True,
        drops_trailing_backslash=False,
    )
    assert scan_line(line, None) == expected, line


@settings(max_examples=50, deadline=None)
@given(
    words=st.lists(st.sampled_from(QUOTED_WORDS), max_size=3),
    opener=st.sampled_from(("'", '"')),
)
def test_a_line_inside_an_open_quote_opens_nothing(
    words: list[str], opener: str
) -> None:
    """Inside an open quote the other quote character is literal."""
    inner = '"' if opener == "'" else "'"
    line = f"{' '.join(words)} {inner}still quoted"
    expected = LineScan(
        quote=opener,
        continues=True,
        drops_trailing_backslash=False,
    )
    assert scan_line(line, opener) == expected, line


@settings(max_examples=50, deadline=None)
@given(
    words=st.lists(st.sampled_from(QUOTED_WORDS), max_size=3),
    opener=st.sampled_from(("'", '"')),
)
def test_a_closed_quote_ends_the_line(words: list[str], opener: str) -> None:
    """A matched pair leaves no quote open and nothing to continue."""
    line = f"echo {opener}{' '.join(words)}{opener}"
    expected = LineScan(quote=None, continues=False, drops_trailing_backslash=False)
    assert scan_line(line, None) == expected, line


@settings(max_examples=50, deadline=None)
@given(
    body_line=st.text(st.characters(min_codepoint=32, max_codepoint=126), max_size=8),
    tail=st.sampled_from(TAIL_COMMANDS),
)
def test_a_quoted_here_document_body_cannot_swallow_later_commands(
    body_line: str, tail: str
) -> None:
    """A quote in the body is data, so the tail stays visible to the reader."""
    assume(body_line != "EOF")
    run = f"cat <<'EOF'\n{body_line}\nEOF\n{tail}"
    assert _logical_lines(run) == ["cat <<'EOF'", tail], run
