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

from tests.heavy_step_scope_contract import (
    LOOM_GATE_STEPS,
    command_words,
    loom_condition_problems,
    loom_tolerance_problems,
    run_tokens,
)
from tests.heavy_workflow_document import (
    HEAVY_JOB,
    LOOM_STEP,
    Job,
    env,
    run_text,
    steps,
)

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

#: Options that make the checker exit before it checks a model. They are valid
#: for a human at a terminal and useless in this step.
_CHECKER_EARLY_EXITS: tuple[str, ...] = ("--help", "-h", "--version")

#: The separator token plus the shortest possible command after it.
_SEPARATOR_AND_ONE_COMMAND: int = 2


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

    The command the checker runs must also reach it: a real argument after the
    separator, and no option before it that ends the checker before it checks
    anything. `--help` would otherwise satisfy the prefix and separator rules
    while printing usage and exiting without running a model.

    Parameters
    ----------
    found : Job
        The parsed heavy job.

    Returns
    -------
    list[str]
        A complaint when the checker invocation, its argument separator, its
        foreground execution, its checked command or its pre-separator options
        are wrong.
    """
    step = _execution_step(found)
    if step is None:
        return [f"the job must have a {LOOM_STEP!r} step"]
    tokens = run_tokens(step)
    if any(token in _FOREGROUND_ONLY_TOKENS for token in tokens):
        return [
            (
                f"the {LOOM_STEP!r} step must run the checker in the "
                "foreground as one command"
            )
        ]
    words = command_words(tokens)
    if words[:4] != ("uv", "run", "--script", _RUNNER):
        return [
            (
                f"the {LOOM_STEP!r} step must run the models under {_RUNNER}, "
                f"its command begins {words[:4]!r}"
            )
        ]
    return _argument_problems(words)


def _argument_problems(words: tuple[str, ...]) -> list[str]:
    """Return the complaints about the checker's own arguments.

    Returns
    -------
    list[str]
        A complaint for a missing separator, an option that exits the checker
        early, or a separator with no command after it.
    """
    if "--" not in words:
        return [f"the {LOOM_STEP!r} step must pass the checker's `--` separator"]
    separator = words.index("--")
    early_exits = [
        option for option in words[4:separator] if option in _CHECKER_EARLY_EXITS
    ]
    if early_exits:
        return [
            (
                f"the {LOOM_STEP!r} step must not pass {early_exits[0]!r} to the "
                "checker before its `--` separator"
            )
        ]
    if len(words) - separator < _SEPARATOR_AND_ONE_COMMAND:
        return [
            (
                f"the {LOOM_STEP!r} step must pass the command the checker "
                "runs after its `--` separator"
            )
        ]
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
    if any(token.startswith(local_prefix) for token in run_tokens(step)):
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


def step_presence_problems(found: Job) -> list[str]:
    """Return a complaint unless each named Loom step appears exactly once.

    The condition and tolerance rules inspect only the steps that survive, so
    removing the compile gate outright would leave them with nothing to
    complain about. The compile step is a gate in its own right: it proves the
    models build before the execution step spends forty minutes on them.

    A duplicate is as bad as a removal. The rules look a step up by name, and
    two lookups can land on different steps: the command checks could pass on
    one while the shell check passes on another, and the job would report
    success with no valid gate at all.

    Parameters
    ----------
    found : Job
        The parsed heavy job.

    Returns
    -------
    list[str]
        One complaint per missing or duplicated named step.
    """
    names = [step.get("name") for step in steps(found, subject=HEAVY_JOB)]
    problems: list[str] = []
    for name in LOOM_GATE_STEPS:
        count = names.count(name)
        if count == 0:
            problems.append(f"the {name!r} step must remain in the {HEAVY_JOB!r} job")
        elif count > 1:
            problems.append(
                f"the {name!r} step must appear once in the {HEAVY_JOB!r} job, "
                f"found {count}"
            )
    return problems
