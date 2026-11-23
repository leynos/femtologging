"""Rules for the pull-request gate that reproduces the scheduled Loom build.

The scheduled ``heavy-tests`` job compiles every target under ``--cfg loom``
before it runs any model. That configuration is a day late by design, so a
library test that refers to a helper compiled out under ``--cfg loom`` can sit
on main overnight. These rules hold the same configuration on every pull
request, as one whole step command, and refuse the shapes that would satisfy a
text search while running nothing: a comment, an ``echo``, a condition, a
tolerated failure or a masked exit status.

Each rule returns one complaint per violation, so the contract test can prove
every judgement against a deliberately broken copy as well as the real file.
The document accessors live in :mod:`tests.make_contract_helpers`.
"""

from __future__ import annotations

import shlex
import typing as typ

import yaml

from tests.make_contract_helpers import mapping, repo_root

type Tokens = tuple[str, ...]
type Workflow = dict[str, object]

#: The workflow and job that own the gate.
CI_WORKFLOW: typ.Final = ".github/workflows/ci.yml"
CI_JOB: typ.Final = "build-test"

#: The gate's step name and the scheduled command it must run verbatim.
CI_LOOM_CLIPPY_STEP: typ.Final = "Loom all-targets Clippy"
LOOM_CLIPPY_COMMAND: typ.Final = (
    'RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold" cargo clippy '
    "--manifest-path rust_extension/Cargo.toml --no-default-features "
    "--all-targets -- -D warnings"
)

#: `--cfg loom` changes every crate's fingerprint, so the gate builds apart
#: from the ordinary lanes rather than invalidating them.
LOOM_CLIPPY_TARGET_DIR: typ.Final = "target/loom"

#: The step the gate must follow, because it resolves dependencies first.
FETCH_STEP: typ.Final = "Fetch locked dependencies"

#: Shell fragments that turn a gate into something other than the command it
#: names, or that mask its exit status. The whole-command comparison already
#: rules each out; naming them makes the refusal legible to a reader.
MASK_FRAGMENTS: typ.Final[Tokens] = ("||", "&&", ";", "echo ", "#")


def job_tolerance_is_acceptable(job: dict[str, object]) -> bool:
    """Report whether a job's failure tolerance is acceptable for a gate.

    An allow-list of shapes, not a deny-list of spellings: a deny-list would
    have to enumerate every constant-true expression. A job may omit the key,
    set an explicit `false`, or consult the matrix, which is how an
    experimental leg is singled out.

    Returns
    -------
    bool
        True when the job's failure would still fail CI.
    """
    if "continue-on-error" not in job:
        return True
    tolerance = job["continue-on-error"]
    if tolerance is False:
        return True
    return isinstance(tolerance, str) and "matrix." in tolerance


def step_tolerance_is_acceptable(step: dict[str, object]) -> bool:
    """Report whether a step's failure tolerance is acceptable for a gate.

    Stricter than the job rule. A matrix expression on a job says which leg is
    experimental; on a required step it would say this gate may fail.

    Returns
    -------
    bool
        True when the step's failure would still fail its job.
    """
    return "continue-on-error" not in step or step["continue-on-error"] is False


def load_ci_workflow() -> Workflow:
    """Return the parsed pull-request workflow.

    Returns
    -------
    Workflow
        The workflow mapping, suitable for a deep-copied mutation.
    """
    document = yaml.safe_load((repo_root() / CI_WORKFLOW).read_text(encoding="utf-8"))
    return mapping(document, subject=CI_WORKFLOW)


def ci_steps(document: Workflow) -> list[dict[str, object]]:
    """Return the job's own step list, so mutations reach the document.

    :func:`tests.make_contract_helpers.objects` returns a new list of the same
    step mappings, which is enough for reading and silently loses a removal,
    so this accessor returns the document's own list instead.

    Returns
    -------
    list[dict[str, object]]
        The job's step mappings, in document order.

    Raises
    ------
    TypeError
        If the job's ``steps`` entry is not a list.
    """
    jobs = mapping(document.get("jobs"), subject=f"{CI_WORKFLOW} jobs")
    build_test = mapping(jobs.get(CI_JOB), subject=f"{CI_JOB} job")
    found_steps = build_test.get("steps")
    if not isinstance(found_steps, list):
        msg = f"{CI_JOB} steps must be a list"
        raise TypeError(msg)
    return typ.cast("list[dict[str, object]]", found_steps)


def loom_step(document: Workflow) -> dict[str, object]:
    """Return the sole Loom Clippy step of a mutable workflow copy.

    Returns
    -------
    dict[str, object]
        The step mapping.
    """
    return next(
        step for step in ci_steps(document) if step.get("name") == CI_LOOM_CLIPPY_STEP
    )


def _command_words(tokens: Tokens) -> Tokens:
    """Return the tokens after any leading ``NAME=value`` assignments.

    Returns
    -------
    Tokens
        The command and its arguments, or ``()`` when the command is nothing
        but assignments.
    """
    for index, token in enumerate(tokens):
        if "=" not in token.split(" ", 1)[0]:
            return tokens[index:]
    return ()


def _normalize_run(run: str) -> str:
    """Return a shell run value without line continuations or layout."""
    return " ".join(run.replace("\\\n", " ").split())


def _run_problems(step: dict[str, object]) -> list[str]:
    """Return the complaints about what the gate step executes.

    The whole ``run`` value is compared, after normalising line continuations
    and layout, so a comment, an ``echo``, or a trailing status mask cannot
    satisfy the contract by leaving the command text in place.

    Returns
    -------
    list[str]
        One complaint per masking, whole-command or program failure.
    """
    run = step.get("run")
    normalized = _normalize_run(run) if isinstance(run, str) else ""
    problems = [
        f"the {CI_LOOM_CLIPPY_STEP!r} step must not contain {fragment!r}"
        for fragment in MASK_FRAGMENTS
        if fragment in normalized
    ]
    if normalized != LOOM_CLIPPY_COMMAND:
        problems.append(
            f"the {CI_LOOM_CLIPPY_STEP!r} step must run exactly the scheduled "
            f"Loom Clippy command, saw {run!r}"
        )
        return problems
    words = _command_words(tuple(shlex.split(normalized)))
    if words[:2] != ("cargo", "clippy"):
        problems.append(
            f"the {CI_LOOM_CLIPPY_STEP!r} step must invoke cargo clippy, its "
            f"command begins {words[:3]!r}"
        )
    return problems


def _gate_problems(build_test: dict[str, object], step: dict[str, object]) -> list[str]:
    """Return the complaints about conditions and failure tolerance.

    Returns
    -------
    list[str]
        One complaint per condition or tolerated failure on the job or step.
    """
    problems: list[str] = []
    if "if" in step:
        problems.append(f"the {CI_LOOM_CLIPPY_STEP!r} step must carry no condition")
    if not step_tolerance_is_acceptable(step):
        problems.append(
            f"the {CI_LOOM_CLIPPY_STEP!r} step must not tolerate its own failure"
        )
    if "if" in build_test:
        problems.append(f"the {CI_JOB} job must carry no condition")
    if not job_tolerance_is_acceptable(build_test):
        problems.append(
            f"the {CI_JOB} job must not tolerate failure outside a matrix leg"
        )
    return problems


def _target_problems(step: dict[str, object]) -> list[str]:
    """Return the complaint when the gate shares the ordinary target directory.

    Returns
    -------
    list[str]
        An empty list when the step sets its own ``target/loom`` directory.
    """
    environment = step.get("env")
    if isinstance(environment, dict) and (
        environment.get("CARGO_TARGET_DIR") == LOOM_CLIPPY_TARGET_DIR
    ):
        return []
    return [
        f"the {CI_LOOM_CLIPPY_STEP!r} step must build into {LOOM_CLIPPY_TARGET_DIR}"
    ]


def _order_problems(found_steps: list[dict[str, object]]) -> list[str]:
    """Return the complaints about the gate's position after the fetch.

    Returns
    -------
    list[str]
        An empty list when the gate follows the locked dependency fetch.
    """
    names = [found.get("name") for found in found_steps]
    if FETCH_STEP not in names:
        return [f"{CI_JOB} must retain its {FETCH_STEP!r} step"]
    if names.index(CI_LOOM_CLIPPY_STEP) <= names.index(FETCH_STEP):
        return [f"the {CI_LOOM_CLIPPY_STEP!r} step must run after {FETCH_STEP!r}"]
    return []


def loom_clippy_problems(document: Workflow) -> list[str]:
    """Return every complaint about the pull-request Loom Clippy gate.

    The pull request must run the exact command the scheduled heavy job runs,
    as a whole step command. A gate rewritten as a comment, an ``echo``, a
    conditional, or a command whose status is masked would satisfy a check that
    only searched the step text, so the comparison is over the whole value.

    Parameters
    ----------
    document : Workflow
        The parsed pull-request workflow.

    Returns
    -------
    list[str]
        One complaint per missing, conditional, masked or misplaced
        requirement. An empty list means the gate is real.
    """
    jobs = mapping(document.get("jobs"), subject=f"{CI_WORKFLOW} jobs")
    build_test = mapping(jobs.get(CI_JOB), subject=f"{CI_JOB} job")
    found_steps = ci_steps(document)
    matches = [step for step in found_steps if step.get("name") == CI_LOOM_CLIPPY_STEP]
    if len(matches) != 1:
        return [f"{CI_JOB} must have exactly one {CI_LOOM_CLIPPY_STEP!r} step"]
    step = matches[0]
    return [
        *_run_problems(step),
        *_gate_problems(build_test, step),
        *_target_problems(step),
        *_order_problems(found_steps),
    ]
