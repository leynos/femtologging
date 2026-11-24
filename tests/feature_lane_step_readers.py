"""Readers for the heavy lane's formatter-and-test step.

The feature-matrix contract asks a handful of questions of one workflow step:
which commands it runs, whether any of them turns off exit-on-error, what
environment it exports, and which shell it actually executes under. Keeping
those readers here lets the contract module stay a list of judgements and
keeps each reader independently testable.

Every reader parses the workflow document rather than scanning the file's
text, so re-indenting the workflow or renaming a nearby step cannot break a
judgement and cannot hide a change either.
"""

from __future__ import annotations

import shlex
import typing as typ

import yaml

from tests.make_contract_helpers import repo_root, sole_workflow_step, workflow_job

if typ.TYPE_CHECKING:
    import collections.abc as cabc

WORKFLOW: typ.Final[str] = ".github/workflows/heavy-tests.yml"
JOB: typ.Final[str] = "heavy"
STEP: typ.Final[str] = "Run formatters and tests"

#: The driver the step delegates the matrix to.
RUNNER: typ.Final[str] = "scripts/run_cargo_feature_tests.py"

#: The driver's invocation, as the whole command.
RUNNER_COMMAND: typ.Final[tuple[str, ...]] = ("uv", "run", "--script", RUNNER)


def step() -> dict[str, object]:
    """Return the heavy job's formatter-and-test step.

    Returns
    -------
        The step mapping from the parsed workflow.
    """
    return sole_workflow_step(WORKFLOW, JOB, STEP)


def run_text() -> str:
    """Return the step's shell script.

    Returns
    -------
        The ``run`` script, with backslash-newline continuations joined so
        each command is one line.

    Raises
    ------
    TypeError
        If the step has no string ``run`` command.
    """
    command = step().get("run")
    if not isinstance(command, str):
        message = f"the {STEP!r} step must have a string `run` command"
        raise TypeError(message)
    return command.replace("\\\n", " ")


#: Tokens that separate one shell command from the next. `shlex` keeps a
#: quoted or escaped copy as part of its word, so `echo "a; b"` stays one
#: command.
_SEPARATORS: typ.Final[frozenset[str]] = frozenset({"&&", "||", ";", "|", "&"})

#: Tokens that redirect a command's streams. The command's argument list is
#: not taken past one, because the names after it belong to the redirection.
_REDIRECTIONS: typ.Final[frozenset[str]] = frozenset({">", ">>", "<", "<<", ">&"})


def _lex(line: str) -> list[str]:
    """Return one line's shell words, with separators kept as their own tokens.

    ``punctuation_chars`` keeps ``;``, ``&&``, ``||`` and ``|`` as individual
    tokens, so a command list is visible rather than being one long word list.
    ``commenters="#"`` reads a trailing ``#`` as a comment, so a comment
    containing an apostrophe does not open a quotation the line never closes;
    a quoted ``#`` stays argument content.

    Returns
    -------
        The line's words and separator tokens, in order.
    """
    lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    lexer.commenters = "#"
    return list(lexer)


def tokenize(script: str) -> list[list[str]]:
    """Return one script's tokens, one entry per line.

    Separators are kept as their own tokens, so a line written as
    ``uv run x || true`` is one entry ending in the suppression rather than
    two commands. That is what lets a contract refuse a suppression suffix;
    a reader that split on separators would see only ``uv run x`` and accept
    it.

    Returns
    -------
        The tokenized lines, in the order the script runs them.
    """
    return [tokens for line in script.splitlines() if (tokens := _lex(line))]


def _split_statement(tokens: cabc.Sequence[str]) -> list[list[str]]:
    """Return one line's tokens, split into its simple commands.

    A redirection ends the command it belongs to, because the words after it
    name the redirection's target rather than the command's arguments.

    Returns
    -------
        The line's simple commands, in order, with empty ones dropped.
    """
    statements: list[list[str]] = []
    current: list[str] = []
    for token in tokens:
        if token in _REDIRECTIONS:
            break
        if token in _SEPARATORS:
            statements.append(current)
            current = []
        else:
            current.append(token)
    statements.append(current)
    return [statement for statement in statements if statement]


def tokenize_statements(script: str) -> list[list[str]]:
    """Return one script's tokens, one entry per simple command.

    A line may hold several commands separated by ``;``, ``&&``, ``||``,
    ``|`` or ``&``. Each is returned separately, because a reader that looked
    only at a line's first word would miss a ``set +e`` written after another
    command on the same line.

    Returns
    -------
        The tokenized simple commands, in the order the script runs them.
    """
    return [
        statement
        for tokens in tokenize(script)
        for statement in _split_statement(tokens)
    ]


def commands() -> list[list[str]]:
    """Return the step's shell script tokens, one entry per command.

    Returns
    -------
        The tokenized commands, in the order the step runs them.
    """
    return tokenize(run_text())


def environment() -> dict[str, object]:
    """Return the step's own environment mapping.

    Returns
    -------
        The ``env`` mapping, or an empty mapping when the step has none.
    """
    found = step().get("env")
    return found if isinstance(found, dict) else {}


def errexit_state(options: cabc.Sequence[str]) -> bool | None:
    """Return the exit-on-error state the option words leave behind.

    Bash processes ``set`` options left to right and the last assignment
    wins, so ``set -e +e`` disables the option even though ``-e`` appears
    first. Short options may be grouped: ``-euo pipefail`` enables ``errexit``
    alongside others, and ``+ex`` disables it alongside others. ``-o`` and
    ``+o`` consume the following word as the option name.

    Returns
    -------
        ``True`` when the words enable ``errexit``, ``False`` when they
        disable it, and ``None`` when they say nothing about it.
    """
    state: bool | None = None
    index = 0
    while index < len(options):
        option = options[index]
        index += 1
        if option in {"-o", "+o"}:
            name = options[index] if index < len(options) else ""
            index += 1
            if name == "errexit":
                state = option == "-o"
            continue
        if option.startswith("--"):
            # A long option is one word, not a group of short ones, so none
            # of its letters is the `e` flag. `--noprofile` does not enable
            # exit-on-error.
            continue
        if option.startswith(("-", "+")) and "e" in option[1:]:
            state = option.startswith("-")
    return state


def disables_errexit(command: list[str]) -> bool:
    """Report whether one command leaves exit-on-error off.

    Only the command words are inspected, so a flag inside a quoted argument
    to another program is not mistaken for the shell builtin.

    Returns
    -------
        True when the command is a ``set`` whose final ``errexit`` assignment
        disables it.
    """
    if not command or command[0] != "set":
        return False
    return errexit_state(command[1:]) is False


def shell_fails_fast(shell: object) -> bool:
    """Report whether a step's effective shell exits on error.

    An absent shell means GitHub's built-in ``bash -e {0}`` on Linux, and the
    built-in ``bash`` and ``sh`` names run with ``-e`` too. A custom template
    has to say so itself, and a later disabling option such as
    ``bash -e +e {0}`` wins over an earlier enabling one, so the whole option
    list is evaluated in order.

    Returns
    -------
        True when the shell is the platform default, a built-in, or a
        template whose final ``errexit`` assignment enables it.
    """
    if shell is None or shell in {"bash", "sh"}:
        return True
    words = shlex.split(str(shell))
    if "{0}" not in words:
        return False
    # Everything after the script operand is an argument to the script, not a
    # shell option, so `bash {0} -e` runs without exit-on-error.
    operand = words.index("{0}")
    return errexit_state(words[1:operand]) is True


def declared_shell(mapping: cabc.Mapping[str, object]) -> object:
    """Return one mapping's ``defaults.run.shell``, or ``None`` when absent.

    The parameter is a ``Mapping`` rather than a ``dict``, which is invariant
    in its value type: the parsed workflow's nested ``dict[str, Any]`` is not
    a ``dict[str, object]``, and the narrower annotation rejects a real
    workflow even though every value it reads is typed.

    Returns
    -------
        The declared shell template, or ``None``.
    """
    defaults = mapping.get("defaults")
    if not isinstance(defaults, dict):
        return None
    run = defaults.get("run")
    if not isinstance(run, dict):
        return None
    return run.get("shell")


def effective_shell() -> object:
    """Return the shell the step actually runs under.

    GitHub resolves ``shell`` from the step, then the job's
    ``defaults.run.shell``, then the workflow's, then the implicit default for
    the runner's platform. Reading only the step would accept an absent key
    while a job-level template replaced the fail-fast default.

    Returns
    -------
        The effective shell template, or ``None`` for the built-in default.

    Raises
    ------
    TypeError
        If the workflow does not parse to a mapping.
    """
    document = yaml.safe_load((repo_root() / WORKFLOW).read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        message = f"{WORKFLOW} must parse to a mapping"
        raise TypeError(message)
    return (
        step().get("shell")
        or declared_shell(workflow_job(WORKFLOW, JOB))
        or declared_shell(typ.cast("dict[str, object]", document))
    )
