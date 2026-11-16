"""Read the scheduled heavy-tests workflow as plain mappings.

The workflow contract test needs to reach individual jobs and steps, both to
inspect them and to mutate a copy. This module parses the YAML once per call
and offers typed accessors, so the rule functions in
:mod:`tests.heavy_workflow_contract_helpers` can name a job or step instead of
walking the document themselves.
"""

from __future__ import annotations

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

# YAML leaves values untyped until a rule inspects them, so the leaf type stays
# `Any`; the aliases only name the container shapes the accessors narrow to.
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

    Parameters
    ----------
    document : Document
        The parsed workflow.
    name : str
        The job key to look up.

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

    Parameters
    ----------
    found : Job
        The job to read.
    subject : str
        A name for the job, used in error messages.

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
    """Return a job's step mappings, in order.

    Parameters
    ----------
    found : Job
        The job to read.
    subject : str
        A name for the job, used in error messages.

    Returns
    -------
    list[Step]
        The step mappings that are mappings themselves, in document order.
    """
    return [
        step for step in raw_steps(found, subject=subject) if isinstance(step, dict)
    ]


def named_step(found: Job, name: str, *, subject: str) -> Step:
    """Return the sole step with ``name``.

    Parameters
    ----------
    found : Job
        The job to search.
    name : str
        The step name to match.
    subject : str
        A name for the job, used in error messages.

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
    """Return a step's shell command, or an empty string when it has none.

    Parameters
    ----------
    step : Step
        The step to read.

    Returns
    -------
    str
        The step's ``run`` command, or ``""`` when it is absent or not text.
    """
    run = step.get("run", "")
    return run if isinstance(run, str) else ""


def env(step_or_job: Step | Job) -> dict[str, typ.Any]:
    """Return a step or job environment mapping, or an empty mapping.

    Parameters
    ----------
    step_or_job : Step | Job
        The mapping whose ``env`` entry is read.

    Returns
    -------
    dict[str, typing.Any]
        The environment mapping, or ``{}`` when there is no mapping.
    """
    environment = step_or_job.get("env")
    return environment if isinstance(environment, dict) else {}


def replace_run(document: Document, step_name: str, command: str) -> None:
    """Replace one heavy-job step's run command.

    Parameters
    ----------
    document : Document
        The workflow to mutate.
    step_name : str
        The name of the heavy-job step to change.
    command : str
        The replacement shell command.
    """
    found = job(document, HEAVY_JOB)
    step = named_step(found, step_name, subject=f"{WORKFLOW} job {HEAVY_JOB!r}")
    step["run"] = command
