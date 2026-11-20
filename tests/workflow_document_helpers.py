"""Readers that discover, load and walk the repository's workflow documents.

The provisioning contract inspects every concrete job of every workflow, so
this module holds the reading half: discovery, parsing (including duplicate-key
rejection), job or step access, and the check that every locally reusable
workflow a job calls exists in the same repository.

The sibling modules each answer one further question about what was read.
`workflow_contract_helpers` states the provisioning rules and consumes these
readers, `workflow_command_helpers` turns a step's ``run`` text into Make goals
and suite detection, and `workflow_runner_helpers` classifies the runner a job
asks for. Keeping them apart leaves each module small and gives each workflow
mutation one place to change.
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

    Notes
    -----
    Reading and parsing happen here, so an unreadable file, invalid YAML or a
    repeated mapping key fails at this boundary instead of returning a
    partial workflow. A document whose root is not a mapping is rejected.
    """
    # The loader is a SafeLoader subclass that rejects duplicate keys.
    document = yaml.load(
        path.read_text("utf-8"),
        Loader=_UniqueKeyLoader,  # ruff: ignore[unsafe-yaml-load] - SafeLoader subclass
    )
    require(isinstance(document, dict), f"{path.name} must declare a mapping")
    return typ.cast("dict[str, typ.Any]", document)


def _job_documents(name: str, document: dict[str, typ.Any]) -> list[tuple[str, Job]]:
    """Return the job mappings declared by one parsed workflow `document`.

    Parameters
    ----------
    name
        The workflow file name, used in failure messages.
    document
        A parsed workflow document.

    Returns
    -------
    list[tuple[str, Job]]
        The job id and document, in file order. A missing jobs mapping or a
        non-mapping job is rejected rather than skipped.
    """
    job_map = document.get("jobs")
    require(isinstance(job_map, dict), f"{name} must declare a jobs mapping")
    jobs: list[tuple[str, Job]] = []
    for job_id, job in typ.cast("dict[str, object]", job_map).items():
        require(isinstance(job, dict), f"{name} job {job_id!r} must be a mapping")
        jobs.append((str(job_id), typ.cast("Job", job)))
    return jobs


def _check_reusable_calls(
    name: str,
    jobs: list[tuple[str, Job]],
    reusable_exists: cabc.Callable[[str], bool],
) -> None:
    """Check that every locally called reusable workflow exists.

    A remote reusable workflow's jobs are not in this repository; its own
    contract owns the tools it installs, so only a local path is verified.

    Parameters
    ----------
    name
        The calling workflow's file name, used in failure messages.
    jobs
        The workflow's jobs.
    reusable_exists
        Reports whether a repository-relative workflow path exists. The caller
        supplies it so this module assumes no repository root of its own. A
        missing local reusable workflow is rejected.
    """
    for job_id, job in jobs:
        reusable = job.get("uses")
        if isinstance(reusable, str) and reusable.startswith("./"):
            require(
                reusable_exists(reusable),
                f"{name} job {job_id!r} calls a missing workflow: {reusable}",
            )


def _concrete_step_jobs(
    name: str, jobs: list[tuple[str, Job]]
) -> list[tuple[str, Job]]:
    """Return the jobs that declare steps of their own.

    Parameters
    ----------
    name
        The workflow file name, used in failure messages.
    jobs
        The workflow's jobs.

    Returns
    -------
    list[tuple[str, Job]]
        Jobs with a step list; reusable-workflow calls are omitted. A
        concrete job without a step list is rejected.
    """
    concrete = []
    for job_id, job in jobs:
        if isinstance(job.get("uses"), str):
            continue
        steps = job.get("steps")
        require(
            isinstance(steps, list),
            f"{name} job {job_id!r} must declare a step list",
        )
        concrete.append((job_id, job))
    return concrete


def workflow_documents() -> list[Path]:
    """Return every repository workflow document, in file-name order.

    Returns
    -------
    list[Path]
        The ``*.yml`` and ``*.yaml`` workflow files, without duplicates. A
        repository that publishes none is rejected rather than silently
        inspected as empty.
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


def repository_workflows() -> list[tuple[str, dict[str, typ.Any]]]:
    """Load every repository workflow through the documented boundaries.

    Returns
    -------
    list[tuple[str, dict[str, typing.Any]]]
        The file name and parsed document, in file-name order. Reading and
        parsing stay in `workflow_documents` and `load_workflow`, so each
        failure is raised at its own boundary.
    """
    return [(path.name, load_workflow(path)) for path in workflow_documents()]


def repository_workflow_exists(reference: str) -> bool:
    """Report whether a repository-relative reusable workflow exists.

    Parameters
    ----------
    reference
        The ``uses`` value of a job calling a local reusable workflow.

    Returns
    -------
    bool
        Whether the referenced file exists in this repository.
    """
    return (ROOT / reference).is_file()


def workflow_jobs(
    documents: cabc.Iterable[tuple[str, dict[str, typ.Any]]],
    *,
    reusable_exists: cabc.Callable[[str], bool],
) -> list[tuple[str, str, Job]]:
    """Return every concrete job of the supplied workflow `documents`.

    Parameters
    ----------
    documents
        File name and parsed document for each workflow to inspect, such as
        ``repository_workflows()``. Parsing stays at that caller's boundary,
        so a malformed document fails where it is read.
    reusable_exists
        Reports whether a repository-relative reusable workflow exists; the
        caller owns that lookup so no global repository root is assumed.

    Returns
    -------
    list[tuple[str, str, Job]]
        The workflow file name, job id and job document, in order. A
        document, job or step list of an unexpected shape, and a missing
        local reusable workflow, are rejected rather than skipped.
    """
    jobs: list[tuple[str, str, Job]] = []
    for name, document in documents:
        entries = _job_documents(name, document)
        _check_reusable_calls(name, entries, reusable_exists)
        jobs.extend(
            (name, job_id, job) for job_id, job in _concrete_step_jobs(name, entries)
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
