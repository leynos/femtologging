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
import yaml.constructor
import yaml.resolver

if typ.TYPE_CHECKING:
    import collections.abc as cabc

ROOT: typ.Final[Path] = Path(__file__).resolve().parents[1]
WORKFLOW_DIR: typ.Final[Path] = ROOT / ".github" / "workflows"
type Job = dict[str, typ.Any]


class _UniqueKeyLoader(yaml.SafeLoader):
    """A safe YAML loader that rejects duplicate mapping keys.

    PyYAML silently keeps the last value of a repeated key, which could hide a
    mutated workflow step. A contract test needs the duplicate to fail loudly
    instead, so the loader is strict while remaining as safe as SafeLoader.
    """


def _construct_unique_mapping(
    loader: _UniqueKeyLoader, node: yaml.nodes.MappingNode
) -> dict[object, object]:
    """Construct a mapping, raising on a repeated key at its first conflict."""
    mapping: dict[object, object] = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node, deep=True)
        if key in mapping:
            context = "while constructing a mapping"
            msg = f"found duplicate key {key!r} on line {key_node.start_mark.line + 1}"
            raise yaml.constructor.ConstructorError(
                context, node.start_mark, msg, key_node.start_mark
            )
        mapping[key] = loader.construct_object(value_node, deep=True)
    return mapping


_UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _construct_unique_mapping
)


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


def load_workflow(path: Path) -> dict[str, typ.Any]:
    """Return the workflow document at `path`, rejecting duplicate keys.

    Parameters
    ----------
    path
        A workflow document.

    Returns
    -------
    dict[str, typing.Any]
        The parsed document. The loader rejects a duplicate mapping key.
    """
    # The loader is a SafeLoader subclass that rejects duplicate keys.
    document = yaml.load(
        path.read_text("utf-8"),
        Loader=_UniqueKeyLoader,  # ruff: ignore[unsafe-yaml-load] - SafeLoader subclass
    )
    require(isinstance(document, dict), f"{path.name} must declare a mapping")
    return typ.cast("dict[str, typ.Any]", document)


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
    document = load_workflow(path)
    job_map = document.get("jobs")
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


def workflow_documents() -> list[Path]:
    """Return every repository workflow document, in file-name order.

    Returns
    -------
    list[Path]
        The ``*.yml`` and ``*.yaml`` workflow files, without duplicates.
    """
    documents = sorted(
        {
            path
            for pattern in ("*.yml", "*.yaml")
            for path in WORKFLOW_DIR.glob(pattern)
        },
        key=lambda path: path.name,
    )
    require(documents, "the repository must publish workflow files")
    return documents


def workflow_jobs(
    paths: cabc.Iterable[Path] | None = None,
) -> list[tuple[str, str, Job]]:
    """Return every concrete job of every repository workflow.

    Parameters
    ----------
    paths
        Workflow documents to inspect. When omitted, every document found by
        `workflow_documents` is read.

    Returns
    -------
    list[tuple[str, str, Job]]
        The workflow file name, job id and job document, in file order.
    """
    documents = workflow_documents() if paths is None else list(paths)
    jobs: list[tuple[str, str, Job]] = []
    for path in documents:
        entries = _job_documents(path)
        _check_reusable_calls(path, entries)
        jobs.extend(
            (path.name, job_id, job)
            for job_id, job in _concrete_step_jobs(path, entries)
        )
    return jobs


def job_steps(document: dict[str, typ.Any], job_id: str) -> list[Job]:
    """Return the steps of `job_id` in a workflow `document`.

    Parameters
    ----------
    document
        A parsed workflow document.
    job_id
        The job whose steps are read.

    Returns
    -------
    list[Job]
        The job's step mappings, in order.
    """
    jobs = document.get("jobs")
    require(isinstance(jobs, dict), "the workflow must declare a jobs mapping")
    job = typ.cast("dict[str, object]", jobs).get(job_id)
    require(isinstance(job, dict), f"the workflow must declare job {job_id!r}")
    steps = typ.cast("dict[str, object]", job).get("steps")
    require(isinstance(steps, list), f"job {job_id!r} must declare a step list")
    listed = typ.cast("list[object]", steps)
    typed_steps = [typ.cast("Job", step) for step in listed if isinstance(step, dict)]
    require(
        len(typed_steps) == len(listed),
        f"job {job_id!r} steps must all be mappings",
    )
    return typed_steps


def step_using(steps: cabc.Sequence[Job], marker: str) -> Job:
    """Return the sole step whose ``uses`` contains `marker`.

    Parameters
    ----------
    steps
        A job's step mappings.
    marker
        Text that identifies the action, such as ``setup-rust``.

    Returns
    -------
    Job
        The matching step.
    """
    matches = [step for step in steps if marker in str(step.get("uses", ""))]
    require(
        len(matches) == 1,
        f"expected exactly one step using {marker!r}, found {len(matches)}",
    )
    return matches[0]
