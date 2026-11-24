"""Deliberate breaks of the scheduled heavy-tests workflow.

Each function mutates a deep copy of the parsed workflow in one way the
contract must report. Keeping them apart from the contract test keeps both
modules below the repository's line limit; the test supplies the deep copy and
the assertion.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.heavy_compile_mutations import COMPILE_MUTATIONS
from tests.heavy_workflow_document import (
    BUILD_STEP,
    HEAVY_JOB,
    LOOM_STEP,
    NOTIFY_JOB,
    SYNC_STEP,
    env,
    job,
    named_step,
    raw_steps,
    replace_run,
    run_text,
    steps,
)

type Document = dict[str, typ.Any]


def _unlocked_sync(document: Document) -> None:
    """Remove ``--locked`` from the sync."""
    replace_run(document, SYNC_STEP, "uv sync --group dev")


def _pip_install_returns(document: Document) -> None:
    """Reintroduce an unpinned pip install."""
    found = job(document, HEAVY_JOB)
    raw_steps(found, subject=HEAVY_JOB).insert(
        0, {"name": "Install tools", "run": "pip install pytest"}
    )


def _sync_after_build(document: Document) -> None:
    """Move the locked sync after the editable build."""
    found_steps = raw_steps(job(document, HEAVY_JOB), subject=HEAVY_JOB)
    sync_at = next(
        at for at, step in enumerate(found_steps) if step.get("name") == SYNC_STEP
    )
    build_at = next(
        at for at, step in enumerate(found_steps) if step.get("name") == BUILD_STEP
    )
    found_steps[sync_at], found_steps[build_at] = (
        found_steps[build_at],
        found_steps[sync_at],
    )


def _sync_without_test_util_config(document: Document) -> None:
    """Drop the coverage configuration from the job and the sync step."""
    found = job(document, HEAVY_JOB)
    env(found).pop("UV_CONFIG_FILE", None)
    env(named_step(found, SYNC_STEP, subject=HEAVY_JOB)).pop("UV_CONFIG_FILE", None)


def _sync_step_disabled(document: Document) -> None:
    """Keep the sync command but disable the step that runs it."""
    found = job(document, HEAVY_JOB)
    named_step(found, SYNC_STEP, subject=HEAVY_JOB)["if"] = "false"


def _build_without_test_util(document: Document) -> None:
    """Build the editable extension without the test hooks."""
    replace_run(
        document,
        BUILD_STEP,
        "source .venv/bin/activate\n"
        "maturin develop --manifest-path rust_extension/Cargo.toml "
        "--features python",
    )


def _notification_without_needs(document: Document) -> None:
    """Remove the notification job's dependency on the heavy job."""
    job(document, NOTIFY_JOB).pop("needs", None)


def _notification_always(document: Document) -> None:
    """Run the notification job on every outcome."""
    job(document, NOTIFY_JOB)["if"] = "always()"


def _notification_or_condition(document: Document) -> None:
    """Accept either a failure or a scheduled run, rather than both."""
    job(document, NOTIFY_JOB)["if"] = "failure() || github.event_name == 'schedule'"


def _notification_inverted_condition(document: Document) -> None:
    """Fail on every event except a scheduled one."""
    job(document, NOTIFY_JOB)["if"] = "failure() && github.event_name != 'schedule'"


def _notification_disabled_condition(document: Document) -> None:
    """Keep both required terms but add a conjunct that disables the job."""
    job(document, NOTIFY_JOB)["if"] = (
        "failure() && github.event_name == 'schedule' && false"
    )


def _notification_step_disabled(document: Document) -> None:
    """Keep the notification commands but disable the step that runs them."""
    found = job(document, NOTIFY_JOB)
    steps(found, subject=NOTIFY_JOB)[0]["if"] = "false"


def _notification_without_issue_write(document: Document) -> None:
    """Withhold the permission the notification job needs."""
    job(document, NOTIFY_JOB)["permissions"] = {"contents": "read"}


def _notification_without_actions_read(document: Document) -> None:
    """Withhold the scope needed to read the failed run's creation time."""
    job(document, NOTIFY_JOB)["permissions"] = {
        "contents": "read",
        "issues": "write",
    }


def _notification_without_run_creation_time(document: Document) -> None:
    """Ask the run for a field other than its creation time."""
    found = job(document, NOTIFY_JOB)
    step = steps(found, subject=NOTIFY_JOB)[0]
    step["run"] = run_text(step).replace("--json createdAt", "--json databaseId")


def _notification_without_run_url(document: Document) -> None:
    """Drop the run URL from the notification."""
    env(job(document, NOTIFY_JOB)).pop("RUN_URL", None)


def _notification_finds_a_label(document: Document) -> None:
    """Find the tracking issue by a label instead of its exact title."""
    found = job(document, NOTIFY_JOB)
    step = steps(found, subject=NOTIFY_JOB)[0]
    step["run"] = run_text(step).replace(
        "gh issue list --repo", "gh issue list --label nightly --repo"
    )


def _notification_searches_bodies(document: Document) -> None:
    """Search issue bodies rather than titles, so a title match is missed."""
    found = job(document, NOTIFY_JOB)
    step = steps(found, subject=NOTIFY_JOB)[0]
    step["run"] = run_text(step).replace("in:title", "in:body")


def _loom_step_no_longer_last(document: Document) -> None:
    """Insert a step after the Loom models."""
    found = job(document, HEAVY_JOB)
    raw_steps(found, subject=HEAVY_JOB).append({"name": "After", "run": "true"})


def _heavy_job_conditioned(document: Document) -> None:
    """Make the whole heavy job skippable."""
    job(document, HEAVY_JOB)["if"] = "false"


def _heavy_job_tolerant(document: Document) -> None:
    """Report a failed heavy job as a successful workflow job."""
    job(document, HEAVY_JOB)["continue-on-error"] = True


def _execute_step_conditioned(document: Document) -> None:
    """Make the model execution step skippable."""
    found = job(document, HEAVY_JOB)
    named_step(found, LOOM_STEP, subject=HEAVY_JOB)["if"] = "false"


def _execute_step_tolerant(document: Document) -> None:
    """Let a failed model run stop failing the job."""
    found = job(document, HEAVY_JOB)
    named_step(found, LOOM_STEP, subject=HEAVY_JOB)["continue-on-error"] = True


def _checker_status_masked(document: Document) -> None:
    """Keep the checker command but discard its exit status."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, LOOM_STEP, subject=HEAVY_JOB)
    step["run"] = f"{run_text(step)} || true"


def _preemption_bound_changed(document: Document) -> None:
    """Run the models under a different search bound than the contract pins."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, LOOM_STEP, subject=HEAVY_JOB)
    env(step)["LOOM_MAX_PREEMPTIONS"] = "2"


def _checker_replaced_with_true(document: Document) -> None:
    """Keep the named step and its bound, but run nothing."""
    found = job(document, HEAVY_JOB)
    named_step(found, LOOM_STEP, subject=HEAVY_JOB)["run"] = "true"


def _checker_backgrounded(document: Document) -> None:
    """Background the checker and reap it with a status-losing `wait`."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, LOOM_STEP, subject=HEAVY_JOB)
    step["run"] = f"{run_text(step).rstrip()} & wait"


def _checker_backgrounded_without_space(document: Document) -> None:
    """Attach the background operator and `wait` to the final argument."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, LOOM_STEP, subject=HEAVY_JOB)
    step["run"] = f"{run_text(step).rstrip()}&wait"


def _checker_asked_for_help(document: Document) -> None:
    """Ask the checker for its usage text, which exits before any model."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, LOOM_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        "--expected rust_extension/tests/heavy/loom-models.txt --",
        "--help --expected rust_extension/tests/heavy/loom-models.txt --",
    )


def _checker_given_no_command(document: Document) -> None:
    """Pass the separator but no command for the checker to run."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, LOOM_STEP, subject=HEAVY_JOB)
    step["run"] = f"{run_text(step).split(' -- ')[0]} --"


def _checker_shell_never_runs(document: Document) -> None:
    """Ask GitHub to parse the step's script without executing it."""
    found = job(document, HEAVY_JOB)
    named_step(found, LOOM_STEP, subject=HEAVY_JOB)["shell"] = "bash -n {0}"


def _job_default_shell_never_runs(document: Document) -> None:
    """Override the shell for every step of the heavy job."""
    job(document, HEAVY_JOB)["defaults"] = {"run": {"shell": "bash -n {0}"}}


def _workflow_default_shell_never_runs(document: Document) -> None:
    """Override the shell for every step of the workflow."""
    document["defaults"] = {"run": {"shell": "bash -n {0}"}}


def _bound_overridden_in_command(document: Document) -> None:
    """Shadow the step's pinned bound with a larger command-local value."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, LOOM_STEP, subject=HEAVY_JOB)
    step["run"] = f"LOOM_MAX_PREEMPTIONS=100 {run_text(step)}"


def _checker_commented_out(document: Document) -> None:
    """Comment the entire script out, so bash runs nothing."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, LOOM_STEP, subject=HEAVY_JOB)
    step["run"] = f"#=disabled {run_text(step)}"


def _duplicate_execution_step(document: Document) -> None:
    """Add a second step with the execution step's name.

    The first carries a valid command and a non-executing shell; the second
    runs `true`. The command checks can accept one and the shell check the
    other, so only a uniqueness rule refuses the pair.
    """
    found = job(document, HEAVY_JOB)
    first = named_step(found, LOOM_STEP, subject=HEAVY_JOB)
    first["shell"] = "bash -n {0}"
    raw_steps(found, subject=HEAVY_JOB).append({"name": LOOM_STEP, "run": "true"})


CORE_MUTATIONS = (
    pytest.param(_unlocked_sync, id="unlocked-sync"),
    pytest.param(_pip_install_returns, id="pip-install-returns"),
    pytest.param(_sync_after_build, id="sync-after-build"),
    pytest.param(_sync_without_test_util_config, id="sync-without-test-util"),
    pytest.param(_sync_step_disabled, id="sync-step-disabled"),
    pytest.param(_build_without_test_util, id="build-without-test-util"),
    pytest.param(_notification_without_needs, id="notification-without-needs"),
    pytest.param(_notification_always, id="notification-always"),
    pytest.param(_notification_or_condition, id="notification-or-condition"),
    pytest.param(
        _notification_inverted_condition,
        id="notification-inverted-condition",
    ),
    pytest.param(
        _notification_disabled_condition,
        id="notification-disabled-condition",
    ),
    pytest.param(_notification_step_disabled, id="notification-step-disabled"),
    pytest.param(_notification_without_issue_write, id="notification-read-only"),
    pytest.param(_notification_without_actions_read, id="notification-no-actions"),
    pytest.param(
        _notification_without_run_creation_time,
        id="notification-wrong-run-field",
    ),
    pytest.param(_notification_without_run_url, id="notification-without-run-url"),
    pytest.param(_notification_finds_a_label, id="notification-by-label"),
    pytest.param(_notification_searches_bodies, id="notification-searches-bodies"),
    pytest.param(_loom_step_no_longer_last, id="loom-not-last"),
    pytest.param(_duplicate_execution_step, id="loom-step-duplicated"),
    pytest.param(_heavy_job_conditioned, id="heavy-job-conditioned"),
    pytest.param(_heavy_job_tolerant, id="heavy-job-tolerant"),
    pytest.param(_execute_step_conditioned, id="execute-step-conditioned"),
    pytest.param(_execute_step_tolerant, id="execute-step-tolerant"),
    pytest.param(_checker_status_masked, id="checker-status-masked"),
    pytest.param(_checker_replaced_with_true, id="checker-replaced-with-true"),
    pytest.param(_checker_backgrounded, id="checker-backgrounded"),
    pytest.param(
        _checker_backgrounded_without_space,
        id="checker-backgrounded-attached",
    ),
    pytest.param(_checker_asked_for_help, id="checker-asked-for-help"),
    pytest.param(_checker_given_no_command, id="checker-given-no-command"),
    pytest.param(_checker_shell_never_runs, id="checker-shell-never-runs"),
    pytest.param(_job_default_shell_never_runs, id="job-default-shell-never-runs"),
    pytest.param(
        _workflow_default_shell_never_runs,
        id="workflow-default-shell-never-runs",
    ),
    pytest.param(_bound_overridden_in_command, id="bound-overridden-in-command"),
    pytest.param(_checker_commented_out, id="checker-commented-out"),
    pytest.param(_preemption_bound_changed, id="preemption-bound-changed"),
)

#: Every deliberate break, core and compile, in one sequence.
MUTATIONS = (*CORE_MUTATIONS, *COMPILE_MUTATIONS)
