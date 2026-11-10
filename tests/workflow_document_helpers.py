"""Readers for workflow documents and their runner labels.

The provisioning contract inspects every job of every repository workflow,
including runner forms expressed as a scalar, a sequence, a mapping or a
matrix. Keeping these document readers away from the Make reachability rules
lets each module stay small and gives the workflow mutations one place to
change.
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

import yaml

ROOT: typ.Final[Path] = Path(__file__).resolve().parents[1]
WORKFLOW_DIR: typ.Final[Path] = ROOT / ".github" / "workflows"
type Job = dict[str, typ.Any]


def require(condition: object, message: str) -> None:
    """Raise an assertion error when `condition` is falsy.

    These helpers are not test modules, so Ruff's test-file assertion boundary
    does not apply to them; an explicit check also keeps the reader usable from
    a plain script.

    Parameters
    ----------
    condition
        The fact the contract depends on.
    message
        What went wrong when the fact does not hold.

    Raises
    ------
    AssertionError
        When `condition` is falsy.
    """
    if not condition:
        msg = message
        raise AssertionError(msg)


def _flatten(value: object) -> list[str]:
    """Return every scalar in a YAML value as text.

    Parameters
    ----------
    value
        A scalar, sequence or mapping from a workflow document.

    Returns
    -------
    list[str]
        The scalar values in document order.
    """
    match value:
        case str():
            return [value]
        case list() | tuple():
            return [text for item in value for text in _flatten(item)]
        case dict():
            return [text for item in value.values() for text in _flatten(item)]
        case _:
            return []


def _matrix_runner_values(matrix: dict[str, typ.Any]) -> list[str]:
    """Return the runner labels a job matrix names.

    Parameters
    ----------
    matrix
        The job's ``strategy.matrix`` mapping.

    Returns
    -------
    list[str]
        Labels from ``include`` entries and from a direct ``os`` key.
    """
    included = matrix.get("include")
    entries = included if isinstance(included, list) else []
    from_include = [
        str(entry["os"])
        for entry in entries
        if isinstance(entry, dict) and "os" in entry
    ]
    return from_include + _flatten(matrix.get("os"))


def runner_values(job: Job) -> list[str]:
    """Return the runner labels and matrix operating systems of `job`."""
    values = _flatten(job.get("runs-on"))
    matrix = job.get("strategy", {}).get("matrix", {})
    if isinstance(matrix, dict):
        values.extend(_matrix_runner_values(matrix))
    return values


def is_linux_job(job: Job) -> bool:
    """Report whether `job` runs on Linux runners.

    A runner expression that no concrete label resolves (such as
    ``${{ matrix.os }}``) is left to the matrix values; an expression with no
    matrix detail is treated as Linux, because the repository's hosted suites
    run there. Non-Linux jobs are a legitimate exception: their platform
    linker replaces ``mold``.

    Returns
    -------
    bool
        Whether the job's concrete runners are Linux.
    """
    concrete = [value for value in runner_values(job) if "${{" not in value]
    if not concrete:
        return True
    joined = " ".join(concrete).lower()
    if any(word in joined for word in ("ubuntu", "linux", "ubicloud")):
        return True
    return not any(word in joined for word in ("windows", "macos"))


def _job_documents(path: Path) -> list[tuple[str, Job]]:
    """Return the job mappings declared by one workflow file.

    Parameters
    ----------
    path
        A workflow document.

    Returns
    -------
    list[tuple[str, Job]]
        The job id and document, in file order.
    """
    document = yaml.safe_load(path.read_text("utf-8"))
    job_map = document.get("jobs") if isinstance(document, dict) else None
    require(isinstance(job_map, dict), f"{path.name} must declare a jobs mapping")
    jobs: list[tuple[str, Job]] = []
    for job_id, job in typ.cast("dict[str, object]", job_map).items():
        require(isinstance(job, dict), f"{path.name} job {job_id!r} must be a mapping")
        jobs.append((str(job_id), typ.cast("Job", job)))
    return jobs


def _check_reusable_calls(path: Path, jobs: list[tuple[str, Job]]) -> None:
    """Check that every locally called reusable workflow exists.

    A remote reusable workflow's jobs are not in this repository; its own
    contract owns the tools it installs, so only a local path is verified.

    Parameters
    ----------
    path
        The calling workflow.
    jobs
        The workflow's jobs.
    """
    for job_id, job in jobs:
        reusable = job.get("uses")
        if isinstance(reusable, str) and reusable.startswith("./"):
            require(
                (ROOT / reusable).is_file(),
                f"{path.name} job {job_id!r} calls a missing workflow: {reusable}",
            )


def _concrete_step_jobs(
    path: Path, jobs: list[tuple[str, Job]]
) -> list[tuple[str, Job]]:
    """Return the jobs that declare steps of their own.

    Parameters
    ----------
    path
        The workflow file, named in failure messages.
    jobs
        The workflow's jobs.

    Returns
    -------
    list[tuple[str, Job]]
        Jobs with a step list; reusable-workflow calls are omitted.
    """
    concrete = []
    for job_id, job in jobs:
        if isinstance(job.get("uses"), str):
            continue
        steps = job.get("steps")
        require(
            isinstance(steps, list),
            f"{path.name} job {job_id!r} must declare a step list",
        )
        concrete.append((job_id, job))
    return concrete


def workflow_jobs() -> list[tuple[str, str, Job]]:
    """Return every concrete job of every repository workflow.

    Returns
    -------
    list[tuple[str, str, Job]]
        The workflow file name, job id and job document, in file order.
    """
    documents = sorted(WORKFLOW_DIR.glob("*.yml"))
    require(documents, "the repository must publish workflow files")
    jobs: list[tuple[str, str, Job]] = []
    for path in documents:
        entries = _job_documents(path)
        _check_reusable_calls(path, entries)
        jobs.extend(
            (path.name, job_id, job)
            for job_id, job in _concrete_step_jobs(path, entries)
        )
    return jobs
