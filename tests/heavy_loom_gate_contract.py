"""Rules for the Loom gates inside the scheduled heavy-tests workflow.

The scheduled job must not be able to report success while no model ran, and
the checker's exit status is the only signal that the models ran. These rules
therefore refuse an explicit condition or a tolerated failure on the job or
either named Loom step, require the checker to run in the foreground as one
command, refuse a checker command whose status is masked, and pin the
preemption bound the successful run used in the step environment.

Each rule returns one complaint per violation, so the contract test can prove
every judgement against a deliberately broken copy as well as the real file.
The document accessors live in :mod:`tests.heavy_workflow_document`.
"""

from __future__ import annotations

import re
import shlex
import typing as typ

from tests.heavy_workflow_document import (
    COMPILE_STEP,
    HEAVY_JOB,
    LOOM_STEP,
    Job,
    env,
    run_text,
    steps,
)

_LOOM_GATE_STEPS: tuple[str, ...] = (COMPILE_STEP, LOOM_STEP)
_CHECKER_MASK_FRAGMENTS: tuple[str, ...] = (
    "||",
    "set +e",
    "; true",
    "| cat",
    "exit 0",
)
_PREEMPTION_BOUND: str = "LOOM_MAX_PREEMPTIONS"
_REQUIRED_PREEMPTION_BOUND: str = "3"
_RUNNER: str = "scripts/run_loom_models.py"

#: Shell control operators and the `wait` builtin. A checker started in the
#: background and reaped by an argument-free `wait` returns zero even when the
#: checker failed, so the step must be one foreground command.
_FOREGROUND_ONLY_TOKENS: tuple[str, ...] = ("&", ";", "|", "&&", "||", "wait")

#: Operators that also bind without surrounding whitespace. `shlex.split` keeps
#: `&wait` attached to the preceding word, so each token is split on these
#: before it is judged.
_ATTACHED_OPERATORS: typ.Final = re.compile(r"([&;|])")


def _run_tokens(step: dict[str, object]) -> tuple[str, ...]:
    """Return the shell command of a step as operator-separated tokens.

    A shell separates `&`, `;` and `|` from their neighbours, with or without
    whitespace, so the lexical tokens are split again on those characters. A
    command ending in `&wait`, or `true;handler`, then exposes the operator and
    the builtin the foreground rule looks for.

    Returns
    -------
    tuple[str, ...]
        The step's command tokens, with line continuations flattened and
        attached control operators separated.
    """
    words = shlex.split(run_text(step).replace("\\\n", " "))
    return tuple(
        part for word in words for part in _ATTACHED_OPERATORS.split(word) if part
    )


def _command_words(tokens: tuple[str, ...]) -> tuple[str, ...]:
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


def loom_condition_problems(found: Job) -> list[str]:
    """Return one complaint per condition on the heavy job or a Loom step.

    A condition is never judged by its value: ``if: false`` and a
    prior-success expression skip the gate equally, and the workflow's step
    conditions are evaluated before the step runs. A Loom gate that can be
    skipped while the job reports success is not a gate.

    Parameters
    ----------
    found : Job
        The parsed heavy job.

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
        if step.get("name") in _LOOM_GATE_STEPS and "if" in step
    )
    return problems


def loom_tolerance_problems(found: Job) -> list[str]:
    """Return one complaint per Loom gate that tolerates its own failure.

    The heavy job may not tolerate failure at all, and neither named Loom step
    may. Without this rule, ``continue-on-error: true`` on the execution step
    would report a failing model run as a successful job: the checker's exit
    status is the only signal the step produces.

    Parameters
    ----------
    found : Job
        The parsed heavy job.

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
        if step.get("name") in _LOOM_GATE_STEPS
        and step.get("continue-on-error", False) is not False
    )
    return problems


def _execution_step(found: Job) -> dict[str, object] | None:
    """Return the Loom execution step, or ``None`` when the job has none."""
    return next(
        (
            step
            for step in steps(found, subject=HEAVY_JOB)
            if step.get("name") == LOOM_STEP
        ),
        None,
    )


def checker_invocation_problems(found: Job) -> list[str]:
    """Return a complaint unless the execution step runs the model checker.

    Rejecting masking fragments is not enough on its own: a step rewritten as
    ``run: "true"`` contains none of them and would satisfy every other rule,
    while running no model. The command must start with the `uv run --script`
    invocation of the committed checker, must pass the ``--`` that separates
    the checker's arguments from Cargo's, and must run in the foreground as a
    single command: a backgrounded checker reaped by `wait` returns zero even
    when it failed.

    Parameters
    ----------
    found : Job
        The parsed heavy job.

    Returns
    -------
    list[str]
        A complaint when the checker invocation, its argument separator or its
        foreground execution is missing.
    """
    step = _execution_step(found)
    if step is None:
        return [f"the job must have a {LOOM_STEP!r} step"]
    tokens = _run_tokens(step)
    if any(token in _FOREGROUND_ONLY_TOKENS for token in tokens):
        return [
            (
                f"the {LOOM_STEP!r} step must run the checker in the "
                "foreground as one command"
            )
        ]
    words = _command_words(tokens)
    if words[:4] != ("uv", "run", "--script", _RUNNER):
        return [
            (
                f"the {LOOM_STEP!r} step must run the models under {_RUNNER}, "
                f"its command begins {words[:4]!r}"
            )
        ]
    if "--" not in words:
        return [f"the {LOOM_STEP!r} step must pass the checker's `--` separator"]
    return []


def checker_mask_problems(found: Job) -> list[str]:
    """Return a complaint when the execution step masks the checker's status.

    ``scripts/run_loom_models.py`` reports success and failure through its exit
    status. A trailing ``|| true``, ``; true``, ``| cat``, ``set +e`` or
    ``exit 0`` keeps the command text intact while discarding that status, so
    the step would pass after a model failure or a missing model.

    Parameters
    ----------
    found : Job
        The parsed heavy job.

    Returns
    -------
    list[str]
        A complaint for each masking fragment in the execution step.
    """
    step = _execution_step(found)
    if step is None:
        return [f"the job must have a {LOOM_STEP!r} step"]
    run = run_text(step)
    return [
        f"the {LOOM_STEP!r} step must not mask the checker's exit status with "
        f"{fragment!r}"
        for fragment in _CHECKER_MASK_FRAGMENTS
        if fragment in run
    ]


def preemption_problems(found: Job) -> list[str]:
    """Return a complaint unless the execution step pins the preemption bound.

    The bound is part of what makes the models finite: without it a model with
    several threads explores until the step's wall-clock timeout. The contract
    pins the value the successful nightly run used, so a change that quietly
    widens or removes the search is visible in review and in the mutation set.

    Parameters
    ----------
    found : Job
        The parsed heavy job.

    Returns
    -------
    list[str]
        A complaint when the execution step does not set the bound to ``3``.
    """
    step = _execution_step(found)
    if step is None:
        return [f"the job must have a {LOOM_STEP!r} step"]
    local_prefix = f"{_PREEMPTION_BOUND}="
    if any(token.startswith(local_prefix) for token in _run_tokens(step)):
        return [
            (
                f"the {LOOM_STEP!r} step must set {_PREEMPTION_BOUND} in the "
                "step environment, not in the command"
            )
        ]
    bound = env(step).get(_PREEMPTION_BOUND)
    if str(bound) != _REQUIRED_PREEMPTION_BOUND:
        return [
            (
                f"the {LOOM_STEP!r} step must set {_PREEMPTION_BOUND} to "
                f"{_REQUIRED_PREEMPTION_BOUND!r}, saw {bound!r}"
            )
        ]
    return []


def loom_gate_problems(found: Job) -> list[str]:
    """Return every complaint about the heavy job's Loom gates.

    Parameters
    ----------
    found : Job
        The parsed heavy job.

    Returns
    -------
    list[str]
        The combined condition, tolerance, masking and bound complaints.
    """
    return [
        *loom_condition_problems(found),
        *loom_tolerance_problems(found),
        *checker_invocation_problems(found),
        *checker_mask_problems(found),
        *preemption_problems(found),
    ]
