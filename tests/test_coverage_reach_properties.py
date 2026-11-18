"""Properties of the workflow reachability closure.

The hand-picked cases in :mod:`tests.test_coverage_reach_contract` are worked
examples; this module states the closure as properties over generated graphs
and compares the reader with an independent breadth-first search. A fixed
table cannot cover every graph shape, so a traversal bug that the table misses
should fail here.
"""

from __future__ import annotations

import typing as typ
from pathlib import Path

from hypothesis import given, settings
from hypothesis import strategies as st

from tests.coverage_reach_helpers import pull_request_reach


def _model_events(document: dict[str, typ.Any]) -> dict[str, typ.Any]:
    """Return a document's declared triggers, read without the reader."""
    declared = document.get("on")
    return declared if isinstance(declared, dict) else {}


def _model_watched(document: dict[str, typ.Any]) -> list[str]:
    """Return the display names a document's `workflow_run` listens for."""
    settings_block = _model_events(document).get("workflow_run")
    if not isinstance(settings_block, dict):
        return []
    names = settings_block.get("workflows")
    return [str(name) for name in names] if isinstance(names, list) else []


def _model_successors(
    documents: dict[str, dict[str, typ.Any]], file_name: str
) -> set[str]:
    """Return every workflow one document can lead to, by a plain reading.

    Written from the workflow language rather than from the reader: a local
    `uses` reference points at another file, and a listener of this workflow's
    display name points at the listener.

    Returns
    -------
    set[str]
        The file names one document can lead to.
    """
    document = documents[file_name]
    jobs = typ.cast("dict[str, typ.Any]", document.get("jobs") or {})
    calls = {
        Path(str(job["uses"])).name
        for job in jobs.values()
        if isinstance(job, dict) and isinstance(job.get("uses"), str)
    }
    display = document.get("name")
    listeners = {
        listener
        for listener, other in documents.items()
        if display is not None and display in _model_watched(other)
    }
    return calls | listeners


def _model_reach(documents: dict[str, dict[str, typ.Any]]) -> set[str]:
    """Return the reached set by breadth-first search over the plain edges."""
    pending = [
        file_name
        for file_name, document in documents.items()
        if "pull_request" in _model_events(document)
    ]
    reached = set(pending)
    while pending:
        for target in _model_successors(documents, pending.pop()):
            if target not in reached:
                reached.add(target)
                pending.append(target)
    return reached


def _unclosed_edges(
    documents: dict[str, dict[str, typ.Any]], reached: set[str]
) -> list[tuple[str, str]]:
    """Return each edge that leaves the reached set."""
    return [
        (source, target)
        for source in sorted(reached)
        for target in sorted(_model_successors(documents, source))
        if target not in reached
    ]


@st.composite
def _graph_document(
    draw: st.DrawFn,
    name: str,
    names: list[str],
    displays: dict[str, str],
) -> dict[str, typ.Any]:
    """Generate one workflow of a graph.

    A listener needs a predecessor, so a single-workflow graph offers no
    `workflow_run` choice at all.

    Returns
    -------
    dict[str, typing.Any]
        One document with a unique display name, one trigger and its calls.
    """
    others = [other for other in names if other != name]
    events = ("pull_request", "schedule", "workflow_run" if others else "schedule")
    event = draw(st.sampled_from(events))
    watched = draw(st.sampled_from(others)) if event == "workflow_run" else None
    declared: dict[str, typ.Any] = {event: None}
    if watched is not None:
        declared["workflow_run"] = {"workflows": [displays[watched]]}
    called = (
        draw(st.lists(st.sampled_from(others), unique=True, max_size=len(others)))
        if others
        else []
    )
    jobs: dict[str, typ.Any] = {
        f"job-{index}": {"uses": f"./.github/workflows/{target}.yml"}
        for index, target in enumerate(called)
    }
    return {
        "name": displays[name],
        "on": declared,
        "jobs": jobs or {"job": {"runs-on": "ubuntu-latest", "steps": []}},
    }


@st.composite
def _workflow_graphs(draw: st.DrawFn) -> dict[str, dict[str, typ.Any]]:
    """Generate a small workflow graph the readers must accept.

    Display names are unique because the reader refuses duplicates, which have
    their own test. Every `workflow_run` names a workflow in the graph, and
    every call points at one, so the generated shapes are all provable.

    Returns
    -------
    dict[str, dict[str, typing.Any]]
        Each generated file name mapped to its document.
    """
    size = draw(st.integers(min_value=1, max_value=4))
    names = [f"wf{index}" for index in range(size)]
    displays = {name: f"WF{index}" for index, name in enumerate(names)}
    return {
        f"{name}.yml": draw(_graph_document(name, names, displays)) for name in names
    }


@settings(max_examples=50, deadline=None)
@given(documents=_workflow_graphs())
def test_reachability_matches_an_independent_closure(
    documents: dict[str, dict[str, typ.Any]],
) -> None:
    """The reader agrees with a breadth-first search over the same edges.

    Equality with the model carries the listener property in both directions:
    a listener is reached exactly when the workflow it watches is. The two
    further assertions state the invariants the publisher contract relies on —
    the reached set is closed under its edges — and that reaching more cannot
    unreach anything.
    """
    reached, remote = pull_request_reach(documents)
    assert remote == set(), remote
    assert reached == _model_reach(documents), documents
    assert not _unclosed_edges(documents, reached), documents
    extended = {
        **documents,
        "extra.yml": {
            "name": "WF-extra",
            "on": {"pull_request": None},
            "jobs": {},
        },
    }
    assert reached <= pull_request_reach(extended)[0], documents
