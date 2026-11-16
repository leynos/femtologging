"""Readers that turn a workflow step's shell text into commands and goals.

A ``run`` body is a small shell script: it may hold several commands on one
line, continue a command across lines with a trailing backslash, quote
comments, and pass options to Make or Cargo. These helpers answer the only two
questions the provisioning contract asks of that text, namely which Make goals
a command names and whether it runs Cargo's test runner. Keeping them apart
from the provisioning rules lets each module stay small and independently
testable.
"""

from __future__ import annotations

import shlex
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

CARGO_SUITE_COMMANDS: typ.Final[frozenset[str]] = frozenset({"test", "nextest"})
# Cargo options that consume the following token as a value, so the subcommand
# search skips both. Options without a value are skipped on their own.
CARGO_VALUE_OPTIONS: typ.Final[frozenset[str]] = frozenset({
    "--bench",
    "--bin",
    "--config",
    "--example",
    "--exclude",
    "--features",
    "--jobs",
    "--manifest-path",
    "--package",
    "--profile",
    "--registry",
    "--target",
    "--target-dir",
    "--test",
    "-F",
    "-Z",
    "-j",
    "-p",
})
CONTROL_OPERATORS: typ.Final[frozenset[str]] = frozenset({
    "&&",
    "||",
    ";",
    "|",
    "&",
    "(",
    ")",
})
# Options whose following token is a value rather than a goal. Long forms are
# usually written with `=`, which the goal filter drops on its own.
VALUE_OPTIONS: typ.Final[frozenset[str]] = frozenset({
    "-C",
    "-f",
    "-I",
    "-o",
    "-W",
    "--directory",
    "--file",
    "--makefile",
    "--include-dir",
    "--old-file",
    "--assume-old",
    "--new-file",
    "--assume-new",
    "--eval",
})


def _line_tokens(line: str) -> list[str]:
    """Return shell-like tokens for `line`, with comments removed."""
    lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    lexer.commenters = "#"
    try:
        return list(lexer)
    except ValueError:
        return []


def _command_segments(tokens: list[str]) -> list[list[str]]:
    """Split `tokens` into commands at shell control operators."""
    segments: list[list[str]] = []
    current: list[str] = []
    for token in tokens:
        if token in CONTROL_OPERATORS:
            segments.append(current)
            current = []
        else:
            current.append(token)
    segments.append(current)
    return segments


def _is_goal(token: str) -> bool:
    """Report whether `token` names a Make goal rather than an option."""
    return not token.startswith("-") and "=" not in token and not token.isdigit()


def _arguments_after_make(tokens: list[str]) -> cabc.Iterator[str]:
    """Yield Make's argument tokens, dropping an option's separate value."""
    index = tokens.index("make") + 1
    while index < len(tokens):
        if tokens[index] in VALUE_OPTIONS:
            index += 2
            continue
        yield tokens[index]
        index += 1


def _segment_goals(tokens: list[str]) -> list[str]:
    """Return the goals of one command's tokens, or none without a `make`."""
    if "make" not in tokens:
        return []
    return [token for token in _arguments_after_make(tokens) if _is_goal(token)]


def _goals(tokens: list[str]) -> list[str]:
    """Return the Make goals of one line's shell `tokens`, in order."""
    return [
        goal
        for segment in _command_segments(tokens)
        for goal in _segment_goals(segment)
    ]


def make_goals(line: str) -> list[str]:
    """Return every Make goal named by the shell `line`, in order.

    A step may run several commands on one line, so the token stream is split
    at shell control operators and each command is read separately. Options,
    option values, variable assignments and bare job counts are not goals.

    Parameters
    ----------
    line
        One line of a workflow step's ``run`` body.

    Returns
    -------
    list[str]
        The goals of every `make` invocation, concatenated in order.

    Examples
    --------
    >>> make_goals("make -C . check-fmt && make test")
    ['check-fmt', 'test']
    """
    return _goals(_line_tokens(line))


def _cargo_command(tokens: list[str]) -> str | None:
    """Return the Cargo subcommand named by `tokens`, if there is one.

    Cargo's grammar puts options before the subcommand, so the subcommand is
    the first token after `cargo` that is not an option. A leading
    ``+<toolchain>`` override is skipped first, as the shell passes it to
    Cargo ahead of every option. Options that take a separate value are
    skipped with it, which is why they are listed above.

    Parameters
    ----------
    tokens
        The shell tokens of one command.

    Returns
    -------
    str | None
        The subcommand word, or ``None`` when `tokens` run no Cargo.
    """
    for at, word in enumerate(tokens):
        if word != "cargo":
            continue
        index = at + 1
        if index < len(tokens) and tokens[index].startswith("+"):
            index += 1
        while index < len(tokens) and tokens[index].startswith("-"):
            index += 2 if tokens[index] in CARGO_VALUE_OPTIONS else 1
        return tokens[index] if index < len(tokens) else None
    return None


def _runs_cargo_suite(tokens: list[str]) -> bool:
    """Report whether any command in `tokens` invokes Cargo's test runner.

    A line may chain several commands, so ``cargo build && cargo test`` must be
    read as a suite step even though the first command is a plain build.

    Returns
    -------
    bool
        Whether any segment runs Cargo's ``test`` or ``nextest`` subcommand.
    """
    return any(
        _cargo_command(segment) in CARGO_SUITE_COMMANDS
        for segment in _command_segments(tokens)
    )


def _continues(line: str) -> bool:
    """Report whether `line` ends in an odd run of backslashes."""
    return (len(line) - len(line.rstrip("\\"))) % 2 == 1


def _logical_lines(run: str) -> list[str]:
    r"""Join the shell lines of `run` that continue with a backslash.

    A trailing backslash joins its line to the next one, so tokenising the
    lines separately would see `make \\` and `  test` as two commands and miss
    the invocation entirely. The join inserts nothing: the shell deletes the
    backslash and its newline before tokenising, so `make te\\` followed by
    `st` runs `make test`. Two backslashes are an escaped backslash and do not
    continue the line.

    Parameters
    ----------
    run
        A step's complete ``run`` body.

    Returns
    -------
    list[str]
        One entry per logical shell command.
    """
    commands: list[str] = []
    pending = ""
    for line in run.splitlines():
        if _continues(line):
            pending += line[:-1]
            continue
        commands.append(pending + line)
        pending = ""
    if pending:
        commands.append(pending)
    return commands
