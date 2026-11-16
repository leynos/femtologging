"""Contract tests for the scheduled heavy-tests workflow.

The nightly lane must install its Python test tools from ``uv.lock`` rather
than resolving them at run time. An unpinned ``pip install`` let the runner
resolve pluggy 1.7.0 while the lock pinned 1.6.0, and the ``stack_info``
snapshot recorded pluggy's own source text, so every scheduled run from
15 October 2026 failed. The lane must also make a red nightly visible: a
separate job comments on, or opens, an issue titled ``heavy-tests nightly
failure``.

Each judgement is driven over a deliberately broken copy of the workflow as
well as the real file, so a check that reports nothing cannot pass by
accident.
"""

from __future__ import annotations

import copy
import typing as typ
from pathlib import Path

import pytest
import yaml

if typ.TYPE_CHECKING:
    import collections.abc as cabc

ROOT: typ.Final[Path] = Path(__file__).resolve().parents[1]
WORKFLOW: typ.Final[str] = ".github/workflows/heavy-tests.yml"
HEAVY_JOB: typ.Final[str] = "heavy"
NOTIFY_JOB: typ.Final[str] = "notify-failure"
SYNC_STEP: typ.Final[str] = "Install locked test dependencies"
BUILD_STEP: typ.Final[str] = "Build editable extension"
LOOM_STEP: typ.Final[str] = "Run Loom models"
SYNC_COMMAND: typ.Final[tuple[str, ...]] = (
    "uv",
    "sync",
    "--frozen",
    "--group",
    "dev",
)
TEST_UTIL_CONFIG: typ.Final[str] = ".github/uv-test-util.toml"
ISSUE_TITLE: typ.Final[str] = "heavy-tests nightly failure"
TEST_UTIL_FEATURES: typ.Final[str] = "--features python,test-util"

type Document = dict[str, typ.Any]
type Job = dict[str, typ.Any]
type Step = dict[str, typ.Any]


def load_workflow() -> Document:
    """Return the parsed heavy-tests workflow.

    Returns
    -------
    Document
        The workflow document.
    """
    document = yaml.safe_load((ROOT / WORKFLOW).read_text(encoding="utf-8"))
    assert isinstance(document, dict), f"{WORKFLOW} must parse to a mapping"
    return typ.cast("Document", document)


def _job(document: Document, name: str) -> Job:
    """Return one job from a workflow document."""
    jobs = document.get("jobs")
    assert isinstance(jobs, dict), f"{WORKFLOW} must define jobs"
    job = jobs.get(name)
    assert isinstance(job, dict), f"{WORKFLOW} must define a {name!r} job"
    return typ.cast("Job", job)


def _raw_steps(job: Job, *, subject: str) -> list[Step]:
    """Return a job's own step list, so a mutation reaches the document."""
    steps = job.get("steps")
    assert isinstance(steps, list), f"{subject} must have a list of steps"
    return typ.cast("list[Step]", steps)


def _steps(job: Job, *, subject: str) -> list[Step]:
    """Return a job's step mappings, in order."""
    return [step for step in _raw_steps(job, subject=subject) if isinstance(step, dict)]


def _named_step(job: Job, name: str, *, subject: str) -> Step:
    """Return the sole step with ``name``."""
    matches = [
        step for step in _steps(job, subject=subject) if step.get("name") == name
    ]
    assert len(matches) == 1, f"{subject} must have one {name!r} step"
    return matches[0]


def _run_text(step: Step) -> str:
    """Return a step's shell command, or an empty string when it has none."""
    run = step.get("run", "")
    return run if isinstance(run, str) else ""


def _env(step_or_job: Step | Job) -> dict[str, typ.Any]:
    """Return a step or job environment mapping, or an empty mapping."""
    environment = step_or_job.get("env")
    return environment if isinstance(environment, dict) else {}


def _install_problems(job: Job) -> list[str]:
    """Return the complaints about how the heavy job installs Python tools.

    Parameters
    ----------
    job
        The parsed heavy job.

    Returns
    -------
    list[str]
        One complaint per way the install is unpinned, misconfigured, or
        ordered after the build that needs it.
    """
    subject = f"{WORKFLOW} job {HEAVY_JOB!r}"
    steps = _steps(job, subject=subject)
    problems: list[str] = [
        "the heavy job must not install Python tools with pip"
        for step in steps
        if "pip install" in _run_text(step)
    ]
    sync_at = next(
        (at for at, step in enumerate(steps) if step.get("name") == SYNC_STEP), None
    )
    if sync_at is None:
        return [*problems, f"the job must have a {SYNC_STEP!r} step"]
    sync = steps[sync_at]
    if tuple(_run_text(sync).split()) != SYNC_COMMAND:
        problems.append(f"the sync must run exactly `{' '.join(SYNC_COMMAND)}`")
    environment = _env(sync)
    if not str(environment.get("UV_CONFIG_FILE", "")).endswith(TEST_UTIL_CONFIG):
        problems.append(f"the sync must use {TEST_UTIL_CONFIG}")
    if "env.pythonLocation" not in str(environment.get("UV_PYTHON", "")):
        problems.append("the sync must select the setup-python interpreter")

    build_at = next(
        (at for at, step in enumerate(steps) if step.get("name") == BUILD_STEP), None
    )
    if build_at is None:
        return [*problems, f"the job must have a {BUILD_STEP!r} step"]
    if build_at < sync_at:
        problems.append("the locked sync must run before the editable build")
    if TEST_UTIL_FEATURES not in " ".join(_run_text(steps[build_at]).split()):
        problems.append(f"the editable build must pass {TEST_UTIL_FEATURES}")
    return problems


def _notification_problems(document: Document) -> list[str]:
    """Return the complaints about the scheduled-failure notification job.

    Parameters
    ----------
    document
        The parsed workflow.

    Returns
    -------
    list[str]
        One complaint per missing dependency, condition, permission, token or
        run detail.
    """
    jobs = document.get("jobs")
    if not isinstance(jobs, dict) or NOTIFY_JOB not in jobs:
        return [f"the workflow must define a {NOTIFY_JOB!r} job"]
    job = _job(document, NOTIFY_JOB)
    subject = f"{WORKFLOW} job {NOTIFY_JOB!r}"
    return [
        *_notification_trigger_problems(job),
        *_notification_permission_problems(job),
        *_notification_environment_problems(job),
        *_notification_command_problems(job, subject=subject),
    ]


def _notification_trigger_problems(job: Job) -> list[str]:
    """Return the complaints about when the notification job runs."""
    problems: list[str] = []
    needs = job.get("needs")
    needed = {needs} if isinstance(needs, str) else set(needs or ())
    if HEAVY_JOB not in needed:
        problems.append("the notification job must depend on the heavy job")
    condition = str(job.get("if", ""))
    if "failure()" not in condition:
        problems.append("the notification job must run only after a failure")
    if "schedule" not in condition:
        problems.append("the notification job must run only for scheduled runs")
    return problems


def _notification_permission_problems(job: Job) -> list[str]:
    """Return the complaints about the notification job's token scopes."""
    permissions = job.get("permissions")
    if not isinstance(permissions, dict):
        return ["the notification job must declare permissions"]
    problems: list[str] = []
    if permissions.get("issues") != "write":
        problems.append("the notification job must be granted issues: write")
    if permissions.get("actions") != "read":
        problems.append("the notification job must be granted actions: read")
    return problems


def _notification_environment_problems(job: Job) -> list[str]:
    """Return the complaints about the details the notification reports."""
    environment = _env(job)
    problems: list[str] = []
    if "github.token" not in str(environment.get("GH_TOKEN", "")):
        problems.append("the notification job must authenticate with github.token")
    if environment.get("ISSUE_TITLE") != ISSUE_TITLE:
        problems.append(f"the tracking issue title must be {ISSUE_TITLE!r}")
    if "github.run_id" not in str(environment.get("RUN_URL", "")):
        problems.append("the notification must name the run URL")
    if "github.sha" not in str(environment.get("RUN_SHA", "")):
        problems.append("the notification must name the commit")
    return problems


def _notification_command_problems(job: Job, *, subject: str) -> list[str]:
    """Return the complaints about the commands the notification runs."""
    runs = [_run_text(step) for step in _steps(job, subject=subject)]
    problems: list[str] = []
    for command, complaint in (
        ("gh issue list", "the job must search for an existing tracking issue"),
        ("gh issue comment", "the job must comment on an existing tracking issue"),
        ("gh issue create", "the job must create the tracking issue when absent"),
    ):
        if not any(command in run for run in runs):
            problems.append(complaint)
    if not any(_reads_run_creation_time(run) for run in runs):
        problems.append("the job must read the run date from the run")
    if any("--label" in run for run in runs):
        problems.append("the tracking issue must not be found by a label")
    return problems


def _reads_run_creation_time(run: str) -> bool:
    """Return whether a command asks GitHub for the run's creation time."""
    return "gh run view" in run and "--json createdAt" in run


def _loom_position_problems(job: Job) -> list[str]:
    """Return the complaint when the Loom step is not the heavy job's last."""
    names = [step.get("name") for step in _steps(job, subject=HEAVY_JOB)]
    if not names or names[-1] != LOOM_STEP:
        return [f"the {LOOM_STEP!r} step must remain last, found {names!r}"]
    return []


def _workflow_problems(document: Document) -> list[str]:
    """Return every complaint about the workflow.

    Parameters
    ----------
    document
        The parsed workflow.

    Returns
    -------
    list[str]
        The combined install, notification and ordering complaints.
    """
    heavy = _job(document, HEAVY_JOB)
    return [
        *_install_problems(heavy),
        *_notification_problems(document),
        *_loom_position_problems(heavy),
    ]


def _replace_run(document: Document, step_name: str, command: str) -> None:
    """Replace one step's run command."""
    job = _job(document, HEAVY_JOB)
    step = _named_step(job, step_name, subject=f"{WORKFLOW} job {HEAVY_JOB!r}")
    step["run"] = command


def _unfrozen_sync(document: Document) -> None:
    """Remove ``--frozen`` from the sync."""
    _replace_run(document, SYNC_STEP, "uv sync --group dev")


def _pip_install_returns(document: Document) -> None:
    """Reintroduce an unpinned pip install."""
    job = _job(document, HEAVY_JOB)
    steps = _raw_steps(job, subject=HEAVY_JOB)
    steps.insert(0, {"name": "Install tools", "run": "pip install pytest"})


def _sync_after_build(document: Document) -> None:
    """Move the locked sync after the editable build."""
    job = _job(document, HEAVY_JOB)
    steps = _raw_steps(job, subject=HEAVY_JOB)
    sync_at = next(at for at, step in enumerate(steps) if step.get("name") == SYNC_STEP)
    build_at = next(
        at for at, step in enumerate(steps) if step.get("name") == BUILD_STEP
    )
    steps[sync_at], steps[build_at] = steps[build_at], steps[sync_at]


def _sync_without_test_util_config(document: Document) -> None:
    """Drop the coverage configuration from the sync step."""
    job = _job(document, HEAVY_JOB)
    step = _named_step(job, SYNC_STEP, subject=HEAVY_JOB)
    _env(step).pop("UV_CONFIG_FILE", None)


def _build_without_test_util(document: Document) -> None:
    """Build the editable extension without the test hooks."""
    _replace_run(
        document,
        BUILD_STEP,
        "source .venv/bin/activate\n"
        "maturin develop --manifest-path rust_extension/Cargo.toml "
        "--features python",
    )


def _notification_without_needs(document: Document) -> None:
    """Remove the notification job's dependency on the heavy job."""
    _job(document, NOTIFY_JOB).pop("needs", None)


def _notification_always(document: Document) -> None:
    """Run the notification job on every outcome."""
    _job(document, NOTIFY_JOB)["if"] = "always()"


def _notification_without_issue_write(document: Document) -> None:
    """Withhold the permission the notification job needs."""
    _job(document, NOTIFY_JOB)["permissions"] = {"contents": "read"}


def _notification_without_actions_read(document: Document) -> None:
    """Withhold the scope needed to read the failed run's creation time."""
    _job(document, NOTIFY_JOB)["permissions"] = {
        "contents": "read",
        "issues": "write",
    }


def _notification_without_run_creation_time(document: Document) -> None:
    """Ask the run for a field other than its creation time."""
    job = _job(document, NOTIFY_JOB)
    step = _steps(job, subject=NOTIFY_JOB)[0]
    step["run"] = _run_text(step).replace("--json createdAt", "--json databaseId")


def _notification_without_run_url(document: Document) -> None:
    """Drop the run URL from the notification."""
    _env(_job(document, NOTIFY_JOB)).pop("RUN_URL", None)


def _notification_finds_a_label(document: Document) -> None:
    """Find the tracking issue by a label instead of its exact title."""
    job = _job(document, NOTIFY_JOB)
    step = _steps(job, subject=NOTIFY_JOB)[0]
    step["run"] = _run_text(step).replace(
        "gh issue list --repo", "gh issue list --label nightly --repo"
    )


def _loom_step_no_longer_last(document: Document) -> None:
    """Insert a step after the Loom models."""
    job = _job(document, HEAVY_JOB)
    _raw_steps(job, subject=HEAVY_JOB).append({"name": "After", "run": "true"})


_MUTATIONS = (
    pytest.param(_unfrozen_sync, id="unfrozen-sync"),
    pytest.param(_pip_install_returns, id="pip-install-returns"),
    pytest.param(_sync_after_build, id="sync-after-build"),
    pytest.param(_sync_without_test_util_config, id="sync-without-test-util"),
    pytest.param(_build_without_test_util, id="build-without-test-util"),
    pytest.param(_notification_without_needs, id="notification-without-needs"),
    pytest.param(_notification_always, id="notification-always"),
    pytest.param(_notification_without_issue_write, id="notification-read-only"),
    pytest.param(_notification_without_actions_read, id="notification-no-actions"),
    pytest.param(
        _notification_without_run_creation_time,
        id="notification-wrong-run-field",
    ),
    pytest.param(_notification_without_run_url, id="notification-without-run-url"),
    pytest.param(_notification_finds_a_label, id="notification-by-label"),
    pytest.param(_loom_step_no_longer_last, id="loom-not-last"),
)


def test_the_real_heavy_workflow_is_accepted() -> None:
    """The committed workflow satisfies every rule, so the mutations prove something."""
    problems = _workflow_problems(load_workflow())
    assert not problems, problems


@pytest.mark.parametrize("mutate", _MUTATIONS)
def test_a_broken_heavy_workflow_is_reported(
    mutate: cabc.Callable[[Document], None],
) -> None:
    """Each deliberate break is reported."""
    document = copy.deepcopy(load_workflow())
    mutate(document)
    assert _workflow_problems(document), "the mutated workflow must be reported"
