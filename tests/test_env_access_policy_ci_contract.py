"""Contracts for the environment-access policy's CI workflow wiring.

The Makefile side of the policy is checked in
`tests/test_env_access_policy_contract.py`. These tests read the workflows
through PyYAML and hold that CI runs the policy gates on every pull request,
unconditionally, and that the heavy workflow delegates the standard Clippy
lanes to Make.
"""

from __future__ import annotations

import copy
import shlex
import typing as typ

import pytest
import yaml

from tests.make_contract_helpers import mapping, objects, repo_root, workflow_job

if typ.TYPE_CHECKING:
    import collections.abc as cabc

type Tokens = tuple[str, ...]
type Workflow = dict[str, object]

#: The workflows that must run the gates, and the commands they must run.
CI_WORKFLOW: typ.Final = ".github/workflows/ci.yml"
HEAVY_TESTS_WORKFLOW: typ.Final = ".github/workflows/heavy-tests.yml"
RUST_TEST_COMMAND: typ.Final = (
    "cargo test --manifest-path rust_extension/Cargo.toml "
    "--no-default-features -- --test-threads=1"
)
REQUIRED_CI_COMMANDS: typ.Final[Tokens] = ("make lint", RUST_TEST_COMMAND)

#: The pull-request gate that reproduces the scheduled heavy job's Clippy
#: configuration. `--cfg loom` changes every crate's fingerprint, so it must
#: own a separate target directory rather than invalidate the ordinary build.
CI_LOOM_CLIPPY_STEP: typ.Final = "Loom all-targets Clippy"
LOOM_CLIPPY_COMMAND: typ.Final = (
    'RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold" cargo clippy '
    "--manifest-path rust_extension/Cargo.toml --no-default-features "
    "--all-targets -- -D warnings"
)
LOOM_CLIPPY_TARGET_DIR: typ.Final = "target/loom"
FETCH_STEP: typ.Final = "Fetch locked dependencies"

#: Shell fragments that turn a gate into something other than the command it
#: names, or that mask its exit status. Each is refused even though the exact
#: command comparison below already rules them out, so a reader sees why.
MASK_FRAGMENTS: typ.Final[Tokens] = ("||", "&&", ";", "echo ", "#")


def test_heavy_workflow_delegates_the_standard_clippy_matrix() -> None:
    """Scenario: a contributor adds Clippy work to the heavy workflow.

    Invariant: the workflow delegates the standard lanes to Make and keeps its
    Loom-only Clippy command separate because its Rust flags alter compilation.
    """
    job = workflow_job(HEAVY_TESTS_WORKFLOW, "heavy")
    steps = objects(job.get("steps"), subject="heavy-tests steps")
    matches = [step for step in steps if step.get("name") == "Run formatters and tests"]
    assert len(matches) == 1, "heavy-tests must have one formatter-and-test step"
    run = matches[0].get("run")
    assert isinstance(run, str), "the heavy-test step must have a shell script"
    normalized_run = " ".join(run.replace("\\\n", " ").split())
    assert "make lint-rust-clippy" in normalized_run, (
        "heavy-tests must delegate standard Clippy lanes to the Make target"
    )
    assert "clippy_feature_sets" not in normalized_run, (
        "heavy-tests must not define a separate standard Clippy matrix"
    )
    assert normalized_run.count("cargo clippy") == 1, (
        "heavy-tests must retain only its separate Loom Clippy invocation"
    )
    assert (
        'RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold" cargo clippy --manifest-path '
        "rust_extension/Cargo.toml --no-default-features --all-targets -- -D warnings"
    ) in normalized_run, "the separate Loom Clippy command must retain its flags"


def workflow_steps(job_name: str) -> list[dict[str, object]]:
    """Return a workflow job's steps.

    Returns
    -------
    list[dict[str, object]]
        Every step in the job, in order.
    """
    job = workflow_job(CI_WORKFLOW, job_name)
    return objects(job.get("steps"), subject=f"{job_name} steps")


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


def load_ci_workflow() -> Workflow:
    """Return the parsed pull-request workflow.

    Returns
    -------
    Workflow
        The workflow mapping, suitable for a deep-copied mutation.
    """
    document = yaml.safe_load((repo_root() / CI_WORKFLOW).read_text(encoding="utf-8"))
    return mapping(document, subject=CI_WORKFLOW)


def _ci_steps(document: Workflow) -> list[dict[str, object]]:
    """Return the ``build-test`` job's own step list, so mutations stick.

    :func:`objects` returns a new list of the same step mappings, which is
    enough for reading and silently loses a removal, so this accessor returns
    the document's own list instead.

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
    build_test = mapping(jobs.get("build-test"), subject="build-test job")
    found_steps = build_test.get("steps")
    if not isinstance(found_steps, list):
        msg = "build-test steps must be a list"
        raise TypeError(msg)
    return typ.cast("list[dict[str, object]]", found_steps)


def _loom_step(document: Workflow) -> dict[str, object]:
    """Return the sole Loom Clippy step of a mutable workflow copy.

    Returns
    -------
    dict[str, object]
        The step mapping.
    """
    return next(
        step for step in _ci_steps(document) if step.get("name") == CI_LOOM_CLIPPY_STEP
    )


def _loom_run_problems(step: dict[str, object]) -> list[str]:
    """Return the complaints about what the gate step executes.

    The whole ``run`` value is compared, after normalising line continuations
    and layout, so a comment, an ``echo``, or a trailing status mask cannot
    satisfy the contract by leaving the command text in place.

    Returns
    -------
    list[str]
        One complaint per whole-command, program or masking failure.
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


def _loom_gate_problems(
    build_test: dict[str, object], step: dict[str, object]
) -> list[str]:
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
        problems.append("the build-test job must carry no condition")
    if not job_tolerance_is_acceptable(build_test):
        problems.append(
            "the build-test job must not tolerate failure outside a matrix leg"
        )
    return problems


def _loom_target_problems(step: dict[str, object]) -> list[str]:
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


def _loom_order_problems(found_steps: list[dict[str, object]]) -> list[str]:
    """Return the complaints about the gate's position after the fetch.

    Returns
    -------
    list[str]
        An empty list when the gate follows the locked dependency fetch.
    """
    names = [found.get("name") for found in found_steps]
    if FETCH_STEP not in names:
        return [f"build-test must retain its {FETCH_STEP!r} step"]
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
    build_test = mapping(jobs.get("build-test"), subject="build-test job")
    found_steps = _ci_steps(document)
    matches = [step for step in found_steps if step.get("name") == CI_LOOM_CLIPPY_STEP]
    if len(matches) != 1:
        return [f"build-test must have exactly one {CI_LOOM_CLIPPY_STEP!r} step"]
    step = matches[0]
    return [
        *_loom_run_problems(step),
        *_loom_gate_problems(build_test, step),
        *_loom_target_problems(step),
        *_loom_order_problems(found_steps),
    ]


def test_ci_triggers_on_pull_requests() -> None:
    """Scenario: a contributor edits the workflow triggers.

    Invariant: the workflow still fires on pull requests. Without it every step
    below remains correct and none of them runs.
    """
    workflow = mapping(
        yaml.safe_load((repo_root() / CI_WORKFLOW).read_text(encoding="utf-8")),
        subject=CI_WORKFLOW,
    )
    triggers = mapping(workflow.get(True, workflow.get("on")), subject="triggers")
    assert "pull_request" in triggers, f"{CI_WORKFLOW} must trigger on pull_request"


@pytest.mark.parametrize("command", REQUIRED_CI_COMMANDS)
def test_ci_runs_each_policy_gate_unconditionally(command: str) -> None:
    """Scenario: a contributor edits the workflow's gate steps.

    Invariant: each gate is some step's whole `run` value, and neither that
    step nor its job carries a condition or tolerates its own failure. The
    value of a condition is never inspected: `if: false` and a push-only
    condition skip the gate equally, and YAML resolves `false` to a boolean
    whose string form is `False`, so no list of spellings is reliable.
    """
    job_name = "build-test"
    job = workflow_job(CI_WORKFLOW, job_name)
    matches = [step for step in workflow_steps(job_name) if step.get("run") == command]
    assert matches, f"{CI_WORKFLOW} must run {command!r} as a step's whole command"
    step = matches[0]
    assert "if" not in step, (
        f"the {command!r} step in job {job_name} must carry no condition"
    )
    assert "if" not in job, f"job {job_name} runs {command!r} but carries a condition"
    assert step_tolerance_is_acceptable(step), (
        f"the {command!r} step in job {job_name} must not tolerate its own failure"
    )
    assert job_tolerance_is_acceptable(job), (
        f"job {job_name} runs {command!r} but tolerates failure outside a matrix leg"
    )


def _removed_loom_step(document: Workflow) -> None:
    """Delete the pull-request Loom Clippy gate."""
    _ci_steps(document).remove(_loom_step(document))


def _echoed_loom_step(document: Workflow) -> None:
    """Print the command instead of running it."""
    _loom_step(document)["run"] = f"echo '{LOOM_CLIPPY_COMMAND}'"


def _commented_loom_step(document: Workflow) -> None:
    """Keep the command text but comment it out."""
    _loom_step(document)["run"] = f"# {LOOM_CLIPPY_COMMAND}"


def _masked_loom_step(document: Workflow) -> None:
    """Run the gate and discard its exit status."""
    _loom_step(document)["run"] = f"{LOOM_CLIPPY_COMMAND} || true"


def _conditional_loom_step(document: Workflow) -> None:
    """Disable the gate with a step condition."""
    _loom_step(document)["if"] = "false"


def _tolerated_loom_step(document: Workflow) -> None:
    """Let the gate fail without failing the job."""
    _loom_step(document)["continue-on-error"] = True


_LOOM_MUTATIONS = (
    pytest.param(_removed_loom_step, id="loom-step-removed"),
    pytest.param(_echoed_loom_step, id="loom-step-echoed"),
    pytest.param(_commented_loom_step, id="loom-step-commented"),
    pytest.param(_masked_loom_step, id="loom-step-masked"),
    pytest.param(_conditional_loom_step, id="loom-step-conditional"),
    pytest.param(_tolerated_loom_step, id="loom-step-tolerated"),
)


def test_the_ci_loom_clippy_gate_runs_the_scheduled_configuration() -> None:
    """Scenario: a contributor prepares a scheduled-configuration change.

    Invariant: the pull request compiles every target under the same
    ``--cfg loom``, feature and all-targets configuration the heavy job uses,
    before the change can merge. The scheduled lane is a day late by design,
    so this gate is what keeps a cfg-gating mismatch out of it.
    """
    problems = loom_clippy_problems(load_ci_workflow())
    assert not problems, problems


@pytest.mark.parametrize("mutate", _LOOM_MUTATIONS)
def test_a_broken_ci_loom_clippy_gate_is_reported(
    mutate: cabc.Callable[[Workflow], None],
) -> None:
    """Scenario: the gate is removed, echoed, commented, masked or disabled.

    Invariant: each deliberate break is reported. The mutations prove the
    rule's silence on the real workflow is a judgement rather than a check
    that never runs.
    """
    document = copy.deepcopy(load_ci_workflow())
    mutate(document)
    assert loom_clippy_problems(document), "the mutated workflow must be reported"
