"""Reachability that decides which workflows a pull request can reach.

A pull-request lane may only run the workflows it can actually reach: a
pull-request trigger, a local reusable-workflow call, or a ``workflow_run``
trigger that watches a workflow already reached. The last route runs forwards:
the workflow naming another in ``workflow_run.workflows`` is the listener, so
it becomes reachable once the watched workflow is, not the other way round.
This reader follows those three routes and returns a call into another
repository separately, because its jobs are not in this repository and so its
environment declarations cannot be proved from here. The workflow readers it
builds on live in :mod:`tests.coverage_contract_helpers`.
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
    """Map each workflow's display name to the file that declares it.

    A ``workflow_run`` trigger names its predecessor by display name, so the
    index has to answer with exactly one file. Two workflows sharing a name
    would let the last one seen win, and a listener could then be judged
    reachable from a workflow that never ran. Such a name is refused instead:
    neither candidate can be proved to be the one GitHub fires.

    Returns
    -------
    dict[str, str]
        Each display name mapped to the one file that declares it.
    """
    by_display: dict[str, str] = {}
    for file_name, document in documents.items():
        if "name" not in document:
            continue
        display = str(document["name"])
        require(
            display not in by_display,
            "a workflow display name must be unique: "
            f"{display!r} is declared by {by_display.get(display)!r} and {file_name!r}",
        )
        by_display[display] = file_name
    return by_display


def _pull_request_documents(
    documents: cabc.Mapping[str, dict[str, typ.Any]],
) -> set[str]:
    """Return the workflows a pull-request event triggers directly."""
    return {
        file_name
        for file_name, document in documents.items()
        if PULL_REQUEST_EVENTS & set(triggers(document))
    }


def _listeners_by_workflow(
    documents: cabc.Mapping[str, dict[str, typ.Any]],
    by_display: cabc.Mapping[str, str],
) -> dict[str, list[str]]:
    """Map each workflow to the workflows whose ``workflow_run`` watches it.

    A ``workflow_run`` trigger names its predecessors, but the edge runs the
    other way for reachability: the document declaring the trigger is the
    listener, and it becomes reachable only once the workflow it watches is.
    Indexing the edges once, before any traversal, keeps that direction in one
    place.

    Parameters
    ----------
    documents
        The repository's workflows, keyed by file name.
    by_display
        Each workflow's display name mapped to the file that declares it.

    Returns
    -------
    dict[str, list[str]]
        The listening file names, keyed by the file they listen for. A
        ``workflow_run`` that names no workflow in this repository is refused,
        because a listener whose predecessor cannot be identified could be
        declared safe without ever being reached.

    Examples
    --------
    >>> _listeners_by_workflow(
    ...     {
    ...         "ci.yml": {"name": "CI", "on": {"pull_request": None}},
    ...         "report.yml": {"on": {"workflow_run": {"workflows": ["CI"]}}},
    ...     },
    ...     {"CI": "ci.yml"},
    ... )
    {'ci.yml': ['report.yml']}
    """
    listeners: dict[str, list[str]] = {}
    for listener, document in documents.items():
        for watched in _workflow_run_names(triggers(document)):
            require(
                watched in by_display,
                f"{listener} listens for unknown: {watched}",
            )
            listeners.setdefault(by_display[watched], []).append(listener)
    return listeners


def _document_targets(
    document: dict[str, typ.Any],
    documents: cabc.Mapping[str, dict[str, typ.Any]],
) -> tuple[list[str], list[str]]:
    """Return the workflows one document calls, and its remote calls.

    A reusable-workflow call into another repository is returned separately
    rather than followed: its jobs live elsewhere, so its environment
    declarations cannot be proved from this repository.

    Returns
    -------
    tuple[list[str], list[str]]
        The local workflows this document calls, and the remote
        reusable-workflow references it calls.
    """
    calls = [(ref, _local_target(ref, documents)) for ref in _job_references(document)]
    remote = [ref for ref, target in calls if target is None]
    local = [target for _, target in calls if target is not None]
    return local, remote


def _enqueue_unreached(target: str, reached: set[str], frontier: list[str]) -> None:
    """Add `target` to the frontier unless it has already been reached."""
    if target not in reached:
        reached.add(target)
        frontier.append(target)


def _reachable_from(
    start: cabc.Iterable[str],
    documents: cabc.Mapping[str, dict[str, typ.Any]],
    listeners: cabc.Mapping[str, list[str]],
) -> tuple[set[str], set[str]]:
    """Follow every edge out of `start` until no unreached workflow remains.

    Parameters
    ----------
    start
        The workflow files reached before any edge is followed.
    documents
        The repository's workflows, keyed by file name.
    listeners
        The listening workflows, keyed by the file they listen for.

    Returns
    -------
    tuple[set[str], set[str]]
        The reachable file names, and every remote reusable-workflow
        reference they call.
    """
    reached = set(start)
    frontier = sorted(reached)
    remote: set[str] = set()
    while frontier:
        file_name = frontier.pop()
        calls, remote_calls = _document_targets(documents[file_name], documents)
        remote.update(remote_calls)
        for target in [*calls, *listeners.get(file_name, [])]:
            _enqueue_unreached(target, reached, frontier)
    return reached, remote


def pull_request_reach(
    documents: cabc.Mapping[str, dict[str, typ.Any]],
) -> tuple[set[str], set[str]]:
    """Return the workflows a pull request can reach, and its remote calls.

    Reachability follows the three routes the workflow language offers: a
    pull-request trigger, a local reusable-workflow call, and a ``workflow_run``
    trigger in another workflow that watches one already reached. A call into
    another repository is returned separately rather than followed: its jobs
    are not in this repository, so its environment declarations cannot be
    proved from here.

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
    listeners = _listeners_by_workflow(documents, _display_name_index(documents))
    return _reachable_from(_pull_request_documents(documents), documents, listeners)
