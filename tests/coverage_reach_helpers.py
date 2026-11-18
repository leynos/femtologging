"""Reachability that decides which workflows a pull request can reach.

A pull-request lane may only run the workflows it can actually reach: a
pull-request trigger, a local reusable-workflow call, or a ``workflow_run``
trigger naming a workflow already reached. This reader follows those three
routes and returns a call into another repository separately, because its jobs
are not in this repository and so its environment declarations cannot be
proved from here. The workflow readers it builds on live in
:mod:`tests.coverage_contract_helpers`.
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

from tests.coverage_contract_helpers import (
    PULL_REQUEST_EVENTS,
    SELF_REPOSITORY,
    triggers,
)
from tests.workflow_document_helpers import require

if typ.TYPE_CHECKING:
    import collections.abc as cabc


def _local_target(reference: str, documents: cabc.Mapping[str, object]) -> str | None:
    """Return the local workflow a job reference names, or ``None``.

    Parameters
    ----------
    reference
        A job's ``uses`` value.
    documents
        The repository's workflows, keyed by file name.

    Returns
    -------
    str | None
        The referenced workflow's file name, or ``None`` for a workflow in
        another repository, whose jobs this repository cannot inspect.
    """
    if reference.startswith("./"):
        file_name = Path(reference).name
        require(file_name in documents, f"a job calls a missing workflow: {reference}")
        return file_name
    require(
        not reference.startswith(f"{SELF_REPOSITORY}/"),
        f"a qualified self-call cannot be verified: {reference}",
    )
    return None


def _workflow_run_names(events: cabc.Mapping[str, typ.Any]) -> list[str]:
    """Return the workflows a ``workflow_run`` trigger listens for."""
    settings = events.get("workflow_run")
    if settings is None:
        return []
    require(isinstance(settings, dict), "a workflow_run trigger must be a mapping")
    names = settings.get("workflows")
    require(isinstance(names, list), "workflow_run must name its workflows")
    return [str(name) for name in names]


def _job_references(document: dict[str, typ.Any]) -> list[str]:
    """Return the reusable workflows the document's jobs call."""
    jobs = document.get("jobs")
    require(isinstance(jobs, dict), "the workflow must declare a jobs mapping")
    return [
        job["uses"]
        for job in typ.cast("dict[str, object]", jobs).values()
        if isinstance(job, dict) and isinstance(job.get("uses"), str)
    ]


def _display_name_index(
    documents: cabc.Mapping[str, dict[str, typ.Any]],
) -> dict[str, str]:
    """Map each workflow's display name to the file that declares it."""
    return {
        str(document["name"]): file_name
        for file_name, document in documents.items()
        if "name" in document
    }


def _pull_request_documents(
    documents: cabc.Mapping[str, dict[str, typ.Any]],
) -> set[str]:
    """Return the workflows a pull-request event triggers directly."""
    return {
        file_name
        for file_name, document in documents.items()
        if PULL_REQUEST_EVENTS & set(triggers(document))
    }


def _document_targets(
    file_name: str,
    document: dict[str, typ.Any],
    documents: cabc.Mapping[str, dict[str, typ.Any]],
    by_display: cabc.Mapping[str, str],
) -> tuple[list[str], list[str]]:
    """Return the workflows one document can reach, and its remote calls.

    A reusable-workflow call into another repository is returned separately
    rather than followed: its jobs live elsewhere, so its environment
    declarations cannot be proved from this repository.

    Returns
    -------
    tuple[list[str], list[str]]
        The workflows this document reaches, and the remote reusable-workflow
        references it calls.
    """
    calls = [(ref, _local_target(ref, documents)) for ref in _job_references(document)]
    remote = [ref for ref, target in calls if target is None]
    targets = [target for _, target in calls if target is not None]
    for display in _workflow_run_names(triggers(document)):
        require(display in by_display, f"{file_name} listens for unknown: {display}")
        targets.append(by_display[display])
    return targets, remote


def _enqueue_unreached(target: str, reached: set[str], frontier: list[str]) -> None:
    """Add `target` to the frontier unless it has already been reached."""
    if target not in reached:
        reached.add(target)
        frontier.append(target)


def pull_request_reach(
    documents: cabc.Mapping[str, dict[str, typ.Any]],
) -> tuple[set[str], set[str]]:
    """Return the workflows a pull request can reach, and its remote calls.

    Reachability follows the three routes the workflow language offers: a
    pull-request trigger, a local reusable-workflow call, and a ``workflow_run``
    trigger naming a workflow already reached. A call into another repository
    is returned separately rather than followed: its jobs are not in this
    repository, so its environment declarations cannot be proved from here.

    Parameters
    ----------
    documents
        The repository's workflows, keyed by file name.

    Returns
    -------
    tuple[set[str], set[str]]
        The reachable file names and the remote reusable-workflow references
        those workflows call, which the caller must account for explicitly.
    """
    by_display = _display_name_index(documents)
    reached = _pull_request_documents(documents)
    remote: set[str] = set()
    frontier = sorted(reached)
    while frontier:
        file_name = frontier.pop()
        document = documents[file_name]
        targets, remote_calls = _document_targets(
            file_name, document, documents, by_display
        )
        remote.update(remote_calls)
        for target in targets:
            _enqueue_unreached(target, reached, frontier)
    return reached, remote
