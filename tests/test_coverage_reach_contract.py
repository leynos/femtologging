"""Contracts for the workflow graph a pull request can reach.

Reachability decides which workflows a pull request may run, so every trigger
shape the reader accepts, every edge it follows and every route it must refuse
is pinned here. The publisher contract builds its isolation claim on this
reader: a workflow outside the reached set cannot be entered by a pull
request, so the reader's edge directions are part of that claim.

The hand-picked cases below are worked examples. The closure itself is stated
as properties over generated graphs, compared against a plain breadth-first
search, because a fixed table cannot cover every graph shape the reader will
meet.
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
    ("document", "refusal"),
    [
        ({"on": "push", True: "push"}, "refuse both `on` spellings together"),
        ({}, "the workflow must declare triggers"),
        ({"on": [1]}, "trigger names must all be strings"),
        ({"on": 7}, "triggers must be a name, a sequence or a mapping"),
    ],
    ids=["both-spellings", "no-trigger", "non-string-name", "not-a-trigger"],
)
def test_the_trigger_reader_refuses_an_ambiguous_document(
    document: dict[object, object], refusal: str
) -> None:
    """Two spellings, no trigger, or an unreadable shape fails loudly.

    Each case names the rule it breaks, so a refusal for an unrelated reason
    cannot stand in for the one under test.
    """
    with pytest.raises(AssertionError, match=refusal):
        triggers(typ.cast("dict[str, typ.Any]", document))


def _fail_closed_graphs() -> list[tuple[dict[str, dict[str, typ.Any]], str]]:
    """Return each unprovable route with the refusal it must raise.

    The qualified self-call case carries a local ``ci.yml`` the reference would
    resolve to if it were read as local at all. Without it, a reader that
    wrongly followed the reference would raise the missing-target error and
    pass the case for the wrong reason.

    Returns
    -------
    list[tuple[dict[str, dict[str, typing.Any]], str]]
        Each synthetic graph and the message its refusal must carry.
    """
    local_ci: dict[str, typ.Any] = {"name": "CI", "on": "schedule", "jobs": {}}
    return [
        (
            {
                "synthetic.yml": {
                    "on": {
                        "pull_request": None,
                        "workflow_run": {"workflows": ["Unknown"]},
                    },
                    "jobs": {},
                }
            },
            "listens for unknown: Unknown",
        ),
        (
            {
                "synthetic.yml": {
                    "on": "pull_request",
                    "jobs": {"build": {"uses": "./.github/workflows/absent.yml"}},
                }
            },
            "a job calls a missing workflow",
        ),
        (
            {
                "ci.yml": local_ci,
                "synthetic.yml": {
                    "on": "pull_request",
                    "jobs": {
                        "build": {
                            "uses": (
                                "leynos/femtologging/.github/workflows/ci.yml@main"
                            )
                        }
                    },
                },
            },
            "a qualified self-call cannot be verified",
        ),
    ]


@pytest.mark.parametrize(
    ("documents", "refusal"),
    _fail_closed_graphs(),
    ids=["unknown-workflow-run", "missing-local", "qualified-self"],
)
def test_the_reachability_reader_fails_closed(
    documents: dict[str, dict[str, typ.Any]], refusal: str
) -> None:
    """An unprovable route is refused rather than declared safe."""
    with pytest.raises(AssertionError, match=refusal):
        pull_request_reach(documents)


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


def test_a_local_reusable_workflow_call_is_followed() -> None:
    """A local `workflow_call` target joins the reached set.

    The missing-target case proves the reference is validated; this one proves
    the traversal continues into it, which is what lets the publisher contract
    inspect a called workflow's jobs.
    """
    documents: dict[str, dict[str, typ.Any]] = {
        "ci.yml": {
            "name": "CI",
            "on": {"pull_request": None},
            "jobs": {"build": {"uses": "./.github/workflows/reusable.yml"}},
        },
        "reusable.yml": {
            "name": "Reusable",
            "on": {"workflow_call": None},
            "jobs": {"measure": {"runs-on": "ubuntu-latest", "steps": []}},
        },
    }
    reached, remote = pull_request_reach(documents)
    assert reached == {"ci.yml", "reusable.yml"}, reached
    assert remote == set(), remote


def test_a_listener_of_a_called_workflow_is_reached() -> None:
    """A `workflow_run` listener of a called workflow is reached through it.

    The edge runs predecessor to listener, and the predecessor here is reached
    only by call: a traversal that stopped after the trigger would miss both.
    """
    documents: dict[str, dict[str, typ.Any]] = {
        "ci.yml": {
            "name": "CI",
            "on": {"pull_request": None},
            "jobs": {"build": {"uses": "./.github/workflows/reusable.yml"}},
        },
        "reusable.yml": {"name": "Reusable", "on": {"workflow_call": None}, "jobs": {}},
        "report.yml": {
            "name": "Report",
            "on": {"workflow_run": {"workflows": ["Reusable"]}},
            "jobs": {},
        },
    }
    reached, _ = pull_request_reach(documents)
    assert reached == {"ci.yml", "reusable.yml", "report.yml"}, reached


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


def test_two_workflows_sharing_a_display_name_are_refused() -> None:
    """An ambiguous display name cannot prove which workflow a listener watches.

    Silently keeping the last declaration would let a listener be judged
    reachable from a workflow that never ran.
    """
    documents: dict[str, dict[str, typ.Any]] = {
        "first.yml": {"name": "CI", "on": {"pull_request": None}, "jobs": {}},
        "second.yml": {"name": "CI", "on": "schedule", "jobs": {}},
    }
    with pytest.raises(AssertionError, match="display name must be unique"):
        pull_request_reach(documents)
