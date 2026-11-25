"""Rules for how the heavy job's Loom steps are actually run.

A workflow can keep a step's command text intact and still prevent it from
running: an overridden `shell` template parses the script without executing
it, and a `defaults.run.shell` does the same for every step of a job or a
workflow. The compile step can likewise echo its command instead of building.
These rules refuse those shapes and provide the shell-aware tokenizer the
execution-step rules share.

The document accessors live in :mod:`tests.heavy_workflow_document`.
"""

from __future__ import annotations

import re
import shlex
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

from tests.heavy_workflow_document import (
    COMPILE_STEP,
    FORMATTERS_STEP,
    HEAVY_JOB,
    LOOM_STEP,
    WORKFLOW,
    Job,
    run_text,
    steps,
)
from tests.make_contract_helpers import command_words, defaults_shell

__all__ = [
    "command_words",
    "loom_condition_problems",
    "loom_tolerance_problems",
    "run_tokens",
    "shell_problems",
]

#: The steps the scheduled job must keep and must not make skippable.
#:
#: `Run formatters and tests` carries the deterministic Rust and Python lanes
#: the issue forbids weakening, so a condition or tolerated failure there is as
#: much a gate failure as one on either Loom step.
GATE_STEPS: tuple[str, ...] = (COMPILE_STEP, LOOM_STEP, FORMATTERS_STEP)

#: Operators that also bind without surrounding whitespace. The shell keeps
#: `&wait` attached to the preceding word, so each token is split on these
#: before it is judged.
_ATTACHED_OPERATORS: typ.Final = re.compile(r"([&;|])")

#: A shell redirection. The operand that follows is not an argument, so a
#: command carrying one cannot be validated as a plain argument vector.
_REDIRECTION: typ.Final = re.compile(r"(?:^|\s)(?:\d*[<>]|&>)")

#: The `cargo test` subcommand arguments the compile step must carry.


def run_tokens(step: dict[str, object]) -> tuple[str, ...]:
    """Return the shell command of a step as operator-separated tokens.

    A shell separates `&`, `;` and `|` from their neighbours, with or without
    whitespace, so the lexical tokens are split again on those characters. A
    command ending in `&wait`, or `true;handler`, then exposes the operator and
    the builtin the foreground rule looks for. An unquoted `#` ends the
    command, so a comment-only script tokenizes to nothing.

    A redirection makes the next word an operand rather than an argument — a
    `> --test` consumes the flag as a filename — so a command containing one
    returns a token no rule accepts.

    Returns
    -------
    tuple[str, ...]
        The step's command tokens, with line continuations flattened and
        attached control operators separated.
    """
    command = run_text(step).strip()
    if _REDIRECTION.search(command.replace("\\\n", " ")):
        return ("<redirect>",)
    if "\n" in command.replace("\\\n", ""):
        # An unescaped newline is a command separator. shlex would discard it
        # and read an assignment line as part of the next command, so return a
        # token no rule accepts.
        return ("\n",)
    lexer = shlex.shlex(command.replace("\\\n", " "), posix=True)
    lexer.whitespace_split = True
    lexer.commenters = "#"
    return tuple(
        part for word in lexer for part in _ATTACHED_OPERATORS.split(word) if part
    )


def shell_problems(document: cabc.Mapping[str, object], found: Job) -> list[str]:
    """Return complaints about overrides of how the Loom steps run.

    A ``shell`` template such as ``bash -n {0}`` makes GitHub parse the script
    without executing it, so the step succeeds while nothing runs. The
    effective shell is the step's own setting, else the job's
    ``defaults.run.shell``, else the workflow's; all three are refused, for
    every named gate step.

    Returns
    -------
    list[str]
        One complaint per scope or step that overrides the shell.
    """
    problems: list[str] = []
    named = {step.get("name"): step for step in steps(found, subject=HEAVY_JOB)}
    for step_name in GATE_STEPS:
        step = named.get(step_name)
        if step is not None and "shell" in step:
            problems.append(f"the {step_name!r} step must not override the shell")
    if defaults_shell(found) is not None:
        problems.append(f"the {HEAVY_JOB!r} job must not set defaults.run.shell")
    if defaults_shell(document) is not None:
        problems.append(f"{WORKFLOW} must not set defaults.run.shell")
    return problems


def loom_condition_problems(found: Job) -> list[str]:
    """Return one complaint per condition on the heavy job or a gate step.

    A condition is never judged by its value: ``if: false`` and a
    prior-success expression skip the gate equally, and the workflow's step
    conditions are evaluated before the step runs. A gate that can be skipped
    while the job reports success is not a gate. `Run formatters and tests`
    carries the deterministic Rust and Python lanes the issue requires be
    kept, so it is covered alongside the two Loom steps.

    Returns
    -------
    list[str]
        A complaint for the job or each named gate step that carries ``if``.
    """
    problems: list[str] = []
    if "if" in found:
        problems.append(f"the {HEAVY_JOB!r} job must carry no condition")
    found_steps = steps(found, subject=HEAVY_JOB)
    problems.extend(
        f"the gate step {step.get('name')!r} must carry no condition"
        for step in found_steps
        if step.get("name") in GATE_STEPS and "if" in step
    )
    return problems


def loom_tolerance_problems(found: Job) -> list[str]:
    """Return one complaint per gate step that tolerates its own failure.

    The heavy job may not tolerate failure at all, and neither may any named
    gate step. Without this rule, ``continue-on-error: true`` on the execution
    step would report a failing model run as a successful job, and the same on
    `Run formatters and tests` would hide a failing Rust or Python lane.

    Returns
    -------
    list[str]
        A complaint for the job or each named gate step that tolerates
        failure.
    """
    problems: list[str] = []
    if found.get("continue-on-error", False) is not False:
        problems.append(f"the {HEAVY_JOB!r} job must not tolerate failure")
    found_steps = steps(found, subject=HEAVY_JOB)
    problems.extend(
        f"the gate step {step.get('name')!r} must not tolerate failure"
        for step in found_steps
        if step.get("name") in GATE_STEPS
        and step.get("continue-on-error", False) is not False
    )
    return problems
