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


def _tokenize(script: str) -> list[list[str]]:
    """Return one script's words, one entry per non-empty line.

    ``shlex`` is enough for this ordering question, which only needs to know
    which line names which program. A ``#`` is read as a comment so a comment
    containing an apostrophe does not open a quotation the line never closes;
    a quoted ``#`` stays argument content.

    Returns
    -------
        The tokenized lines, in the order the script runs them.
    """
    return [
        tokens
        for line in script.splitlines()
        if (tokens := shlex.split(line, comments=True))
    ]


def commands() -> list[list[str]]:
    """Return the step's shell script tokens, one entry per line.

    Whether a failed command fails the step is asked of bash in the contract,
    not of this reader; these tokens are only used to say which program a line
    names and in what order.

    Returns
    -------
        The tokenized lines, in the order the step runs them.
    """
    return _tokenize(run_text())


def environment() -> dict[str, object]:
    """Return the step's own environment mapping.

    Returns
    -------
        The ``env`` mapping, or an empty mapping when the step has none.
    """
    found = step().get("env")
    return found if isinstance(found, dict) else {}


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
