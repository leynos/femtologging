"""Contracts for the environment-access policy's CI workflow wiring.

The Makefile side of the policy is checked in
`tests/test_env_access_policy_contract.py`. These tests read the workflows
through PyYAML and hold that CI runs the policy gates on every pull request,
unconditionally, and that the heavy workflow delegates the standard Clippy
lanes to Make.
"""

from __future__ import annotations

import typing as typ

import pytest
import yaml

from tests.make_contract_helpers import mapping, objects, repo_root, workflow_job

type Tokens = tuple[str, ...]

#: The workflows that must run the gates, and the commands they must run.
CI_WORKFLOW: typ.Final = ".github/workflows/ci.yml"
HEAVY_TESTS_WORKFLOW: typ.Final = ".github/workflows/heavy-tests.yml"
RUST_TEST_COMMAND: typ.Final = (
    "cargo test --manifest-path rust_extension/Cargo.toml "
    "--no-default-features -- --test-threads=1"
)
REQUIRED_CI_COMMANDS: typ.Final[Tokens] = ("make lint", RUST_TEST_COMMAND)


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
        'RUSTFLAGS="--cfg loom" cargo clippy --manifest-path '
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
