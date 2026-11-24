"""Contracts for the environment-access policy's CI workflow wiring.

The Makefile side of the policy is checked in
`tests/test_env_access_policy_contract.py`. These tests read the workflows
through PyYAML and hold that CI runs the policy gates on every pull request,
unconditionally, and that the heavy workflow delegates the standard Clippy
lanes to Make. The pull-request gate that reproduces the scheduled Loom
configuration has its own rules in `tests/ci_loom_gate_contract.py`; this
module supplies the mutations that prove those rules bite.
"""

from __future__ import annotations

import copy
import typing as typ

import pytest
import yaml

from tests.ci_gate_scope_contract import (
    job_tolerance_is_acceptable,
    step_tolerance_is_acceptable,
)
from tests.ci_loom_gate_contract import (
    LOOM_CLIPPY_COMMAND,
    Workflow,
    ci_steps,
    load_ci_workflow,
    loom_clippy_problems,
    loom_step,
)
from tests.make_contract_helpers import mapping, objects, repo_root, workflow_job

if typ.TYPE_CHECKING:
    import collections.abc as cabc

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
    ci_steps(document).remove(loom_step(document))


def _echoed_loom_step(document: Workflow) -> None:
    """Print the command instead of running it."""
    loom_step(document)["run"] = f"echo '{LOOM_CLIPPY_COMMAND}'"


def _commented_loom_step(document: Workflow) -> None:
    """Keep the command text but comment it out."""
    loom_step(document)["run"] = f"# {LOOM_CLIPPY_COMMAND}"


def _masked_loom_step(document: Workflow) -> None:
    """Run the gate and discard its exit status."""
    loom_step(document)["run"] = f"{LOOM_CLIPPY_COMMAND} || true"


def _conditional_loom_step(document: Workflow) -> None:
    """Disable the gate with a step condition."""
    loom_step(document)["if"] = "false"


def _tolerated_loom_step(document: Workflow) -> None:
    """Let the gate fail without failing the job."""
    loom_step(document)["continue-on-error"] = True


def _tolerated_on_every_leg(document: Workflow) -> None:
    """Tolerate failure on every leg, including the required ones.

    The expression reads a matrix property, so a predicate that only searched
    for ``matrix.`` would accept it, but logical OR makes it true everywhere.
    """
    jobs = mapping(document.get("jobs"), subject=f"{CI_WORKFLOW} jobs")
    build_test = mapping(jobs.get("build-test"), subject="build-test job")
    build_test["continue-on-error"] = "${{ true || matrix.experimental }}"


def _split_across_lines(document: Workflow) -> None:
    """Put the assignment on its own line, so the shell runs it separately."""
    loom_step(document)["run"] = (
        'RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold"\n'
        "cargo clippy --manifest-path rust_extension/Cargo.toml "
        "--no-default-features --all-targets -- -D warnings"
    )


def _every_leg_experimental(document: Workflow) -> None:
    """Make the matrix itself experimental on every leg.

    The tolerance is the accepted bare selector, so only the matrix values
    decide whether any required leg is left to fail.

    Raises
    ------
    TypeError
        If the workflow copy no longer has a matrix with `include` rows.
    """
    jobs = mapping(document.get("jobs"), subject=f"{CI_WORKFLOW} jobs")
    build_test = mapping(jobs.get("build-test"), subject="build-test job")
    strategy = mapping(build_test.get("strategy"), subject="build-test strategy")
    matrix = mapping(strategy.get("matrix"), subject="build-test matrix")
    included = matrix.get("include")
    if not isinstance(included, list):
        msg = "build-test matrix must carry include rows"
        raise TypeError(msg)
    for row in included:
        mapping(row, subject="matrix include row")["experimental"] = True


def _checker_never_runs(document: Workflow) -> None:
    """Ask GitHub to parse the gate's script without executing it."""
    loom_step(document)["shell"] = "bash -n {0}"


def _job_default_shell_never_runs(document: Workflow) -> None:
    """Override the shell for every step of the job."""
    jobs = mapping(document.get("jobs"), subject=f"{CI_WORKFLOW} jobs")
    build_test = mapping(jobs.get("build-test"), subject="build-test job")
    build_test["defaults"] = {"run": {"shell": "bash -n {0}"}}


def _workflow_default_shell_never_runs(document: Workflow) -> None:
    """Override the shell for every step of the workflow."""
    document["defaults"] = {"run": {"shell": "bash -n {0}"}}


def _excluded_only_required_leg(document: Workflow) -> None:
    """Exclude the one leg that would still fail the job."""
    jobs = mapping(document.get("jobs"), subject=f"{CI_WORKFLOW} jobs")
    build_test = mapping(jobs.get("build-test"), subject="build-test job")
    strategy = mapping(build_test.get("strategy"), subject="build-test strategy")
    matrix = mapping(strategy.get("matrix"), subject="build-test matrix")
    included = matrix.get("include")
    if not isinstance(included, list):
        msg = "build-test matrix must carry include rows"
        raise TypeError(msg)
    for row in included:
        mapping(row, subject="matrix include row")["experimental"] = True
    matrix["exclude"] = [{"experimental": False}]


def _environment(container: dict[str, object], subject: str) -> dict[str, object]:
    """Return a container's environment mapping, creating an empty one.

    Returns
    -------
    dict[str, object]
        The writable environment mapping; :func:`mapping` raises ``TypeError``
        when an existing `env` value is not one.
    """
    container.setdefault("env", {})
    return mapping(container["env"], subject=subject)


def _encoded_flags_override(document: Workflow) -> None:
    """Displace the inline flags with an encoded value Cargo prefers."""
    environment = _environment(loom_step(document), "Loom Clippy step env")
    environment["CARGO_ENCODED_RUSTFLAGS"] = "-Clink-arg=-fuse-ld=mold"


def _job_encoded_flags_override(document: Workflow) -> None:
    """Set the encoded flags on the job, outranking the step's own."""
    jobs = mapping(document.get("jobs"), subject=f"{CI_WORKFLOW} jobs")
    build_test = mapping(jobs.get("build-test"), subject="build-test job")
    environment = _environment(build_test, "build-test job env")
    environment["CARGO_ENCODED_RUSTFLAGS"] = "-Clink-arg=-fuse-ld=mold"


def _workflow_encoded_flags_override(document: Workflow) -> None:
    """Set the encoded flags for the whole workflow."""
    environment = _environment(document, f"{CI_WORKFLOW} env")
    environment["CARGO_ENCODED_RUSTFLAGS"] = "-Clink-arg=-fuse-ld=mold"


_LOOM_MUTATIONS = (
    pytest.param(_removed_loom_step, id="loom-step-removed"),
    pytest.param(_echoed_loom_step, id="loom-step-echoed"),
    pytest.param(_commented_loom_step, id="loom-step-commented"),
    pytest.param(_masked_loom_step, id="loom-step-masked"),
    pytest.param(_conditional_loom_step, id="loom-step-conditional"),
    pytest.param(_tolerated_loom_step, id="loom-step-tolerated"),
    pytest.param(_tolerated_on_every_leg, id="job-tolerated-every-leg"),
    pytest.param(_split_across_lines, id="loom-step-split-across-lines"),
    pytest.param(_every_leg_experimental, id="every-leg-experimental"),
    pytest.param(_encoded_flags_override, id="encoded-flags-step"),
    pytest.param(_job_encoded_flags_override, id="encoded-flags-job"),
    pytest.param(
        _workflow_encoded_flags_override,
        id="encoded-flags-workflow",
    ),
    pytest.param(_excluded_only_required_leg, id="excluded-only-required-leg"),
    pytest.param(_checker_never_runs, id="loom-step-shell-never-runs"),
    pytest.param(_job_default_shell_never_runs, id="job-default-shell-never-runs"),
    pytest.param(
        _workflow_default_shell_never_runs,
        id="workflow-default-shell-never-runs",
    ),
)


def test_the_ci_loom_clippy_gate_runs_the_scheduled_configuration() -> None:
    """Scenario: a contributor prepares a scheduled-configuration change.

    Invariant: the pull request compiles every target under the same
    ``--cfg loom``, feature and all-targets configuration the heavy job uses,
    before the change can merge. The scheduled lane runs once a day, and the
    19 November 2026 run failed before any model executed, so this gate is
    what keeps a cfg-gating mismatch from waiting for the nightly.
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


def test_a_mixed_hyphenated_matrix_keeps_a_required_leg() -> None:
    """Scenario: a matrix names its tolerance property with a hyphen.

    Invariant: a bare selector over a matrix with both required and
    experimental legs is accepted, and a compound expression over the same
    matrix is refused. GitHub permits hyphens in dot-property names, so a
    selector that rejected them would refuse a valid configuration.
    """
    job: dict[str, object] = {
        "continue-on-error": "${{ matrix.allow-failure }}",
        "strategy": {"matrix": {"allow-failure": [False, True]}},
    }
    assert job_tolerance_is_acceptable(job), "a mixed matrix must keep its required leg"
    compound: dict[str, object] = {
        **job,
        "continue-on-error": "${{ true || matrix.allow-failure }}",
    }
    assert not job_tolerance_is_acceptable(compound), (
        "a constant-true expression must be refused"
    )
