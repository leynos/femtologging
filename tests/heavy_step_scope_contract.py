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
    HEAVY_JOB,
    LOOM_STEP,
    WORKFLOW,
    Job,
    run_text,
    steps,
)

#: The steps the scheduled job must keep and must not make skippable.
LOOM_GATE_STEPS: tuple[str, ...] = (COMPILE_STEP, LOOM_STEP)

#: Operators that also bind without surrounding whitespace. The shell keeps
#: `&wait` attached to the preceding word, so each token is split on these
#: before it is judged.
_ATTACHED_OPERATORS: typ.Final = re.compile(r"([&;|])")

#: The `cargo test` subcommand arguments the compile step must carry.
_HEAVY_TARGET: tuple[str, ...] = ("--test", "heavy")
_NO_RUN: str = "--no-run"

#: Shell control operators and the `wait` builtin: a compile step must be one
#: foreground command whose failure the job sees.
_CONTROL_TOKENS: tuple[str, ...] = ("&", ";", "|", "&&", "||", "wait")

#: Options that make Cargo print help instead of compiling.
_EARLY_EXITS: tuple[str, ...] = ("--help", "-h")


def run_tokens(step: dict[str, object]) -> tuple[str, ...]:
    """Return the shell command of a step as operator-separated tokens.

    A shell separates `&`, `;` and `|` from their neighbours, with or without
    whitespace, so the lexical tokens are split again on those characters. A
    command ending in `&wait`, or `true;handler`, then exposes the operator and
    the builtin the foreground rule looks for. An unquoted `#` ends the
    command, so a comment-only script tokenizes to nothing.

    Returns
    -------
    tuple[str, ...]
        The step's command tokens, with line continuations flattened and
        attached control operators separated.
    """
    lexer = shlex.shlex(run_text(step).replace("\\\n", " "), posix=True)
    lexer.whitespace_split = True
    lexer.commenters = "#"
    return tuple(
        part for word in lexer for part in _ATTACHED_OPERATORS.split(word) if part
    )


def command_words(tokens: tuple[str, ...]) -> tuple[str, ...]:
    """Return the tokens after any leading ``NAME=value`` assignments.

    Returns
    -------
    tuple[str, ...]
        The command and its arguments, or ``()`` when the command is nothing
        but assignments.
    """
    for index, token in enumerate(tokens):
        if "=" not in token.split(" ", 1)[0]:
            return tokens[index:]
    return ()


def _defaults_shell(container: cabc.Mapping[str, object]) -> object:
    """Return a workflow or job ``defaults.run.shell`` value, if one is set."""
    defaults = container.get("defaults")
    if not isinstance(defaults, dict):
        return None
    run = defaults.get("run")
    if not isinstance(run, dict):
        return None
    return run.get("shell")


def shell_problems(document: cabc.Mapping[str, object], found: Job) -> list[str]:
    """Return complaints about overrides of how the Loom steps run.

    A ``shell`` template such as ``bash -n {0}`` makes GitHub parse the script
    without executing it, so the step succeeds while nothing runs. The
    effective shell is the step's own setting, else the job's
    ``defaults.run.shell``, else the workflow's; all three are refused, for
    both named Loom steps.

    Returns
    -------
    list[str]
        One complaint per scope or step that overrides the shell.
    """
    problems: list[str] = []
    named = {step.get("name"): step for step in steps(found, subject=HEAVY_JOB)}
    for step_name in LOOM_GATE_STEPS:
        step = named.get(step_name)
        if step is not None and "shell" in step:
            problems.append(f"the {step_name!r} step must not override the shell")
    if _defaults_shell(found) is not None:
        problems.append(f"the {HEAVY_JOB!r} job must not set defaults.run.shell")
    if _defaults_shell(document) is not None:
        problems.append(f"{WORKFLOW} must not set defaults.run.shell")
    return problems


def loom_condition_problems(found: Job) -> list[str]:
    """Return one complaint per condition on the heavy job or a Loom step.

    A condition is never judged by its value: ``if: false`` and a
    prior-success expression skip the gate equally, and the workflow's step
    conditions are evaluated before the step runs. A Loom gate that can be
    skipped while the job reports success is not a gate.

    Returns
    -------
    list[str]
        A complaint for the job or each named Loom step that carries ``if``.
    """
    problems: list[str] = []
    if "if" in found:
        problems.append(f"the {HEAVY_JOB!r} job must carry no condition")
    found_steps = steps(found, subject=HEAVY_JOB)
    problems.extend(
        f"the Loom step {step.get('name')!r} must carry no condition"
        for step in found_steps
        if step.get("name") in LOOM_GATE_STEPS and "if" in step
    )
    return problems


def loom_tolerance_problems(found: Job) -> list[str]:
    """Return one complaint per Loom gate that tolerates its own failure.

    The heavy job may not tolerate failure at all, and neither named Loom step
    may. Without this rule, ``continue-on-error: true`` on the execution step
    would report a failing model run as a successful job.

    Returns
    -------
    list[str]
        A complaint for the job or each named Loom step that tolerates
        failure.
    """
    problems: list[str] = []
    if found.get("continue-on-error", False) is not False:
        problems.append(f"the {HEAVY_JOB!r} job must not tolerate failure")
    found_steps = steps(found, subject=HEAVY_JOB)
    problems.extend(
        f"the Loom step {step.get('name')!r} must not tolerate failure"
        for step in found_steps
        if step.get("name") in LOOM_GATE_STEPS
        and step.get("continue-on-error", False) is not False
    )
    return problems


def compile_command_problems(found: Job) -> list[str]:
    """Return a complaint unless the compile step runs the heavy test build.

    A step that keeps its name and its flags but echoes the command compiles
    nothing, and no other rule reads that step's program. The command must be
    a real ``cargo test`` for the heavy target with ``--no-run``.

    Returns
    -------
    list[str]
        A complaint when the compile step's program, target or ``--no-run``
        flag is missing.
    """
    step = next(
        (
            step
            for step in steps(found, subject=HEAVY_JOB)
            if step.get("name") == COMPILE_STEP
        ),
        None,
    )
    if step is None:
        return [f"the job must have a {COMPILE_STEP!r} step"]
    words = command_words(run_tokens(step))
    return _compile_words_problems(words)


def _compile_words_problems(words: tuple[str, ...]) -> list[str]:
    """Return the complaints about the compile step's command tokens.

    Returns
    -------
    list[str]
        A complaint for a masked or backgrounded command, an early-exit
        option, a program other than ``cargo test``, a missing heavy target or
        a missing ``--no-run``.
    """
    if any(token in _CONTROL_TOKENS for token in words):
        return [
            (
                f"the {COMPILE_STEP!r} step must run one foreground command "
                "whose status is not masked"
            )
        ]
    if any(option in _EARLY_EXITS for option in words):
        return [f"the {COMPILE_STEP!r} step must not ask Cargo for help"]
    if words[:2] != ("cargo", "test"):
        return [
            (
                f"the {COMPILE_STEP!r} step must run `cargo test`, its command "
                f"begins {words[:3]!r}"
            )
        ]
    # Cargo's own arguments end at `--`; anything after it belongs to the
    # test binary, so `--test heavy --no-run` there selects no target and
    # compiles nothing.
    cargo_arguments = words[: words.index("--")] if "--" in words else words
    if not _contains_in_order(cargo_arguments, _HEAVY_TARGET):
        return [f"the {COMPILE_STEP!r} step must name the heavy target"]
    if _NO_RUN not in cargo_arguments:
        return [f"the {COMPILE_STEP!r} step must compile without running"]
    return []


def _contains_in_order(tokens: tuple[str, ...], wanted: tuple[str, ...]) -> bool:
    """Return whether `wanted` appears in `tokens` as a contiguous run.

    Returns
    -------
    bool
        True when the run is present.
    """
    width = len(wanted)
    return any(
        tokens[start : start + width] == wanted
        for start in range(len(tokens) - width + 1)
    )
