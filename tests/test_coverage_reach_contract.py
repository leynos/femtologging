"""Contracts for the workflow graph a pull request can reach.

Reachability decides which workflows a pull request may run, so every trigger
shape the reader accepts, every edge it follows and every route it must refuse
is pinned here. The publisher contract builds its isolation claim on this
reader: a workflow outside the reached set cannot be entered by a pull
request, so the reader's edge directions are part of that claim.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.coverage_contract_helpers import triggers
from tests.coverage_reach_helpers import pull_request_reach


@pytest.mark.parametrize(
    ("document", "expected"),
    [
        ({"on": "push"}, {"push": None}),
        ({"on": ["push", "pull_request"]}, {"push": None, "pull_request": None}),
        ({"on": {"push": {"branches": ["main"]}}}, {"push": {"branches": ["main"]}}),
        ({True: "push"}, {"push": None}),
    ],
    ids=["scalar", "sequence", "mapping", "resolved-boolean-key"],
)
def test_the_trigger_reader_accepts_each_declared_shape(
    document: dict[object, object], expected: dict[str, object]
) -> None:
    """Scalar, sequence, mapping and PyYAML's boolean `on` are all read."""
    assert triggers(typ.cast("dict[str, typ.Any]", document)) == expected, document


@pytest.mark.parametrize(
    "document",
    [
        {"on": "push", True: "push"},
        {},
        {"on": [1]},
        {"on": 7},
    ],
    ids=["both-spellings", "no-trigger", "non-string-name", "not-a-trigger"],
)
def test_the_trigger_reader_refuses_an_ambiguous_document(
    document: dict[object, object],
) -> None:
    """Two spellings, no trigger, or an unreadable shape fails loudly."""
    with pytest.raises(AssertionError):
        triggers(typ.cast("dict[str, typ.Any]", document))


@pytest.mark.parametrize(
    "document",
    [
        {
            "on": {
                "pull_request": None,
                "workflow_run": {"workflows": ["Unknown"]},
            },
            "jobs": {},
        },
        {
            "on": "pull_request",
            "jobs": {"build": {"uses": "./.github/workflows/absent.yml"}},
        },
        {
            "on": "pull_request",
            "jobs": {
                "build": {"uses": "leynos/femtologging/.github/workflows/ci.yml@main"}
            },
        },
    ],
    ids=["unknown-workflow-run", "missing-local", "qualified-self"],
)
def test_the_reachability_reader_fails_closed(document: dict[str, typ.Any]) -> None:
    """An unprovable route is refused rather than declared safe."""
    with pytest.raises(AssertionError):
        pull_request_reach({"synthetic.yml": document})


def test_a_remote_call_is_reported_rather_than_followed() -> None:
    """A call into another repository is surfaced for explicit verification."""
    reference = "example/shared/.github/workflows/called.yml@sha"
    document: dict[str, typ.Any] = {
        "on": "pull_request",
        "jobs": {"build": {"uses": reference}},
    }
    reached, remote = pull_request_reach({"synthetic.yml": document})
    assert reached == {"synthetic.yml"}, reached
    assert remote == {reference}, remote


def test_a_listener_is_reached_once_the_workflow_it_watches_is() -> None:
    """A `workflow_run` edge runs forwards, from predecessor to listener.

    Reading the edge backwards would report only `ci.yml` here, and any
    isolation check built on that set would accept `coverage.yml` even when it
    declared the protected environment.
    """
    documents: dict[str, dict[str, typ.Any]] = {
        "ci.yml": {"name": "CI", "on": "pull_request", "jobs": {}},
        "coverage.yml": {
            "name": "Coverage",
            "on": {"workflow_run": {"workflows": ["CI"]}},
            "jobs": {},
        },
    }
    reached, remote = pull_request_reach(documents)
    assert reached == {"ci.yml", "coverage.yml"}, reached
    assert remote == set(), remote


def test_a_listener_of_an_unreached_workflow_stays_unreached() -> None:
    """Listening for a scheduled workflow does not put a listener in reach."""
    documents: dict[str, dict[str, typ.Any]] = {
        "ci.yml": {"name": "CI", "on": "pull_request", "jobs": {}},
        "nightly.yml": {"name": "Nightly", "on": "schedule", "jobs": {}},
        "report.yml": {
            "name": "Report",
            "on": {"workflow_run": {"workflows": ["Nightly"]}},
            "jobs": {},
        },
    }
    reached, _ = pull_request_reach(documents)
    assert reached == {"ci.yml"}, reached
