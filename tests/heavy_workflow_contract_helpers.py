"""Helpers for the scheduled heavy-tests workflow contract.

The nightly lane must install its Python test tools from ``uv.lock`` rather
than resolving them at run time, and it must make a red run visible. These
helpers parse the workflow and return one complaint per violated rule, so the
contract test can prove each judgement against a deliberately broken copy as
well as the real file.
"""

from __future__ import annotations

import re
import typing as typ
from pathlib import Path

import yaml

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
_REQUIRED_COMMANDS: typ.Final[tuple[tuple[str, str], ...]] = (
    ("gh issue list", "the job must search for an existing tracking issue"),
    ("gh issue comment", "the job must comment on an existing tracking issue"),
    ("gh issue create", "the job must create the tracking issue when absent"),
)
_FAILURE_CONDITION_PATTERN: re.Pattern[str] = re.compile(r"^failure\(\)$")
_SCHEDULE_CONDITION_PATTERN: re.Pattern[str] = re.compile(
    r"""^github\.event_name\s*==\s*['"]schedule['"]$"""
)

type Document = dict[str, typ.Any]
type Job = dict[str, typ.Any]
type Step = dict[str, typ.Any]


def load_workflow() -> Document:
    """Return the parsed heavy-tests workflow.

    Returns
    -------
    Document
        The workflow document.

    Raises
    ------
    TypeError
        If the workflow does not parse to a mapping.
    """
    document = yaml.safe_load((ROOT / WORKFLOW).read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        message = f"{WORKFLOW} must parse to a mapping"
        raise TypeError(message)
    return typ.cast("Document", document)


def job(document: Document, name: str) -> Job:
    """Return one job from a workflow document.

    Returns
    -------
    Job
        The named job's mapping.

    Raises
    ------
    TypeError
        If the document has no jobs or no job named ``name``.
    """
    jobs = document.get("jobs")
    if not isinstance(jobs, dict):
        message = f"{WORKFLOW} must define jobs"
        raise TypeError(message)
    found = jobs.get(name)
    if not isinstance(found, dict):
        message = f"{WORKFLOW} must define a {name!r} job"
        raise TypeError(message)
    return typ.cast("Job", found)


def raw_steps(found: Job, *, subject: str) -> list[Step]:
    """Return a job's own step list, so a mutation reaches the document.

    Returns
    -------
    list[Step]
        The job's step mappings, in document order.

    Raises
    ------
    TypeError
        If the job's ``steps`` entry is not a list.
    """
    found_steps = found.get("steps")
    if not isinstance(found_steps, list):
        message = f"{subject} must have a list of steps"
        raise TypeError(message)
    return typ.cast("list[Step]", found_steps)


def steps(found: Job, *, subject: str) -> list[Step]:
    """Return a job's step mappings, in order."""
    return [
        step for step in raw_steps(found, subject=subject) if isinstance(step, dict)
    ]


def named_step(found: Job, name: str, *, subject: str) -> Step:
    """Return the sole step with ``name``.

    Returns
    -------
    Step
        The matching step's mapping.

    Raises
    ------
    ValueError
        If the job has zero or several steps with that name.
    """
    matches = [
        step for step in steps(found, subject=subject) if step.get("name") == name
    ]
    if len(matches) != 1:
        message = f"{subject} must have one {name!r} step"
        raise ValueError(message)
    return matches[0]


def run_text(step: Step) -> str:
    """Return a step's shell command, or an empty string when it has none."""
    run = step.get("run", "")
    return run if isinstance(run, str) else ""


def env(step_or_job: Step | Job) -> dict[str, typ.Any]:
    """Return a step or job environment mapping, or an empty mapping."""
    environment = step_or_job.get("env")
    return environment if isinstance(environment, dict) else {}


def replace_run(document: Document, step_name: str, command: str) -> None:
    """Replace one heavy-job step's run command."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, step_name, subject=f"{WORKFLOW} job {HEAVY_JOB!r}")
    step["run"] = command


def install_problems(found: Job) -> list[str]:
    """Return the complaints about how the heavy job installs Python tools.

    Parameters
    ----------
    found
        The parsed heavy job.

    Returns
    -------
    list[str]
        One complaint per way the install is unpinned, misconfigured, or
        ordered after the build that needs it.
    """
    subject = f"{WORKFLOW} job {HEAVY_JOB!r}"
    found_steps = steps(found, subject=subject)
    problems: list[str] = [
        "the heavy job must not install Python tools with pip"
        for step in found_steps
        if "pip install" in run_text(step)
    ]
    sync_at = next(
        (at for at, step in enumerate(found_steps) if step.get("name") == SYNC_STEP),
        None,
    )
    if sync_at is None:
        return [*problems, f"the job must have a {SYNC_STEP!r} step"]
    problems.extend(_sync_problems(found_steps[sync_at]))
    build_at = next(
        (at for at, step in enumerate(found_steps) if step.get("name") == BUILD_STEP),
        None,
    )
    if build_at is None:
        return [*problems, f"the job must have a {BUILD_STEP!r} step"]
    if build_at < sync_at:
        problems.append("the locked sync must run before the editable build")
    if TEST_UTIL_FEATURES not in " ".join(run_text(found_steps[build_at]).split()):
        problems.append(f"the editable build must pass {TEST_UTIL_FEATURES}")
    return problems


def _sync_problems(sync: Step) -> list[str]:
    """Return the complaints about the locked dependency sync."""
    problems: list[str] = []
    if tuple(run_text(sync).split()) != SYNC_COMMAND:
        problems.append(f"the sync must run exactly `{' '.join(SYNC_COMMAND)}`")
    environment = env(sync)
    if not str(environment.get("UV_CONFIG_FILE", "")).endswith(TEST_UTIL_CONFIG):
        problems.append(f"the sync must use {TEST_UTIL_CONFIG}")
    if "env.pythonLocation" not in str(environment.get("UV_PYTHON", "")):
        problems.append("the sync must select the setup-python interpreter")
    return problems


def notification_problems(document: Document) -> list[str]:
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
    found = job(document, NOTIFY_JOB)
    subject = f"{WORKFLOW} job {NOTIFY_JOB!r}"
    return [
        *_trigger_problems(found),
        *_permission_problems(found),
        *_environment_problems(found),
        *_command_problems(found, subject=subject),
    ]


def _trigger_problems(found: Job) -> list[str]:
    """Return the complaints about when the notification job runs."""
    problems: list[str] = []
    needs = found.get("needs")
    needed = {needs} if isinstance(needs, str) else set(needs or ())
    if HEAVY_JOB not in needed:
        problems.append("the notification job must depend on the heavy job")
    problems.extend(_condition_problems(str(found.get("if", ""))))
    return problems


def _condition_problems(condition: str) -> list[str]:
    """Return the complaints about the notification job's ``if`` expression.

    Both requirements must be separate conjuncts. ``failure() ||
    github.event_name == 'schedule'`` contains the same text as the conjunction
    it is meant to be, but it also fires after a successful scheduled run and
    after a failed manual one, so substring presence alone cannot accept it.
    Each conjunct is matched whole, so an inverted comparison such as
    ``github.event_name != 'schedule'`` is rejected too.

    Returns
    -------
    list[str]
        One complaint per requirement the expression fails to impose.
    """
    condition = _strip_expression_wrapper(condition)
    terms = [_peel_parens(term) for term in condition.split("&&")]
    problems: list[str] = []
    if "||" in condition:
        problems.append("the notification condition must join its terms with &&")
    if not any(_FAILURE_CONDITION_PATTERN.match(term) for term in terms):
        problems.append("the notification job must run only after a failure")
    if not any(_SCHEDULE_CONDITION_PATTERN.match(term) for term in terms):
        problems.append("the notification job must run only for scheduled runs")
    return problems


def _strip_expression_wrapper(condition: str) -> str:
    """Return the condition without a ``${{ ... }}`` wrapper, if it has one."""
    condition = condition.strip()
    if condition.startswith("${{") and condition.endswith("}}"):
        return condition[3:-2].strip()
    return condition


def _peel_parens(term: str) -> str:
    """Return a condition term without whitespace or wrapping parentheses."""
    term = term.strip()
    while term.startswith("(") and term.endswith(")"):
        term = term[1:-1].strip()
    return term


def _permission_problems(found: Job) -> list[str]:
    """Return the complaints about the notification job's token scopes."""
    permissions = found.get("permissions")
    if not isinstance(permissions, dict):
        return ["the notification job must declare permissions"]
    problems: list[str] = []
    if permissions.get("issues") != "write":
        problems.append("the notification job must be granted issues: write")
    if permissions.get("actions") != "read":
        problems.append("the notification job must be granted actions: read")
    return problems


def _environment_problems(found: Job) -> list[str]:
    """Return the complaints about the details the notification reports."""
    environment = env(found)
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


def _command_problems(found: Job, *, subject: str) -> list[str]:
    """Return the complaints about the commands the notification runs."""
    runs = [run_text(step) for step in steps(found, subject=subject)]
    return [
        *_missing_command_problems(runs),
        *_run_detail_problems(runs),
        *_issue_lookup_problems(runs),
    ]


def _missing_command_problems(runs: list[str]) -> list[str]:
    """Return one complaint for each required ``gh`` command that never runs."""
    return [
        complaint
        for command, complaint in _REQUIRED_COMMANDS
        if not any(command in run for run in runs)
    ]


def _run_detail_problems(runs: list[str]) -> list[str]:
    """Return the complaints about the run details the notification reports."""
    if not any(_reads_run_creation_time(run) for run in runs):
        return ["the job must read the run date from the run"]
    return []


def _reads_run_creation_time(run: str) -> bool:
    """Return whether a command asks GitHub for the run's creation time."""
    return "gh run view" in run and "--json createdAt" in run


def _issue_lookup_problems(runs: list[str]) -> list[str]:
    """Return the complaints about how the notification finds the issue."""
    problems: list[str] = []
    if any("--label" in run for run in runs):
        problems.append("the tracking issue must not be found by a label")
    if not any(_searches_issue_titles(run) for run in runs):
        problems.append("the issue lookup must search titles on the server")
    return problems


def _searches_issue_titles(run: str) -> bool:
    """Return whether a command narrows the issue lookup to title matches."""
    return "gh issue list" in run and "in:title" in run


def loom_position_problems(found: Job) -> list[str]:
    """Return the complaint when the Loom step is not the heavy job's last."""
    names = [step.get("name") for step in steps(found, subject=HEAVY_JOB)]
    if not names or names[-1] != LOOM_STEP:
        return [f"the {LOOM_STEP!r} step must remain last, found {names!r}"]
    return []


def workflow_problems(document: Document) -> list[str]:
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
    heavy = job(document, HEAVY_JOB)
    return [
        *install_problems(heavy),
        *notification_problems(document),
        *loom_position_problems(heavy),
    ]
