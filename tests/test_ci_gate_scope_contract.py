"""Contract tests for the matrix-tolerance rules a gate is judged by.

A gate whose `continue-on-error` reads a bare matrix property is only a gate
if at least one surviving leg still fails. The rules live in
`tests/ci_gate_scope_contract.py`; these tests hold their verdicts on the
matrix shapes GitHub actually expands, including the case-insensitive names
GitHub compares and the exclusions that can remove the last required leg.
"""

from __future__ import annotations

import typing as typ

from tests.ci_gate_scope_contract import job_tolerance_is_acceptable


def test_a_mixed_hyphenated_matrix_keeps_a_required_leg() -> None:
    """Scenario: a matrix names its tolerance property with a hyphen.

    Invariant: a bare selector over a matrix with both required and
    experimental legs is accepted, and a compound expression over the same
    matrix is refused. GitHub permits hyphens in dot-property names, so a
    selector that rejected them would refuse a valid configuration.
    """
    job: dict[str, object] = {
        "continue-on-error": "${{ matrix.allow-failure }}",
        "strategy": {"matrix": {"allow-failure": [False, True]}},
    }
    assert job_tolerance_is_acceptable(job), "a mixed matrix must keep its required leg"
    compound: dict[str, object] = {
        **job,
        "continue-on-error": "${{ true || matrix.allow-failure }}",
    }
    assert not job_tolerance_is_acceptable(compound), (
        "a constant-true expression must be refused"
    )


def test_exclusion_alone_can_remove_the_last_required_leg() -> None:
    """Scenario: `exclude` removes the only required axis combination.

    Invariant: the same tolerance is accepted before the exclusion and refused
    after it. The axis values are untouched, so only the exclusion changes the
    verdict; a checker that ignored `exclude` would accept both.
    """
    job: dict[str, object] = {
        "continue-on-error": "${{ matrix.experimental }}",
        "strategy": {"matrix": {"experimental": [False, True]}},
    }
    assert job_tolerance_is_acceptable(job), (
        "the required axis combination must be accepted before it is excluded"
    )
    strategy = typ.cast("dict[str, object]", job["strategy"])
    matrix = typ.cast("dict[str, object]", strategy["matrix"])
    matrix["exclude"] = [{"experimental": False}]
    assert not job_tolerance_is_acceptable(job), (
        "excluding the required combination must leave no required gate"
    )


def test_a_mixed_case_exclusion_removes_the_required_leg() -> None:
    """Scenario: `exclude` spells the axis name in different case.

    Invariant: GitHub compares matrix names case-insensitively, so an
    `EXPERIMENTAL` exclusion removes the `experimental` leg there. A helper
    comparing names case-sensitively would leave the required leg in place,
    accept the gate, and miss that the surviving leg tolerates failure.
    """
    job: dict[str, object] = {
        "continue-on-error": "${{ matrix.experimental }}",
        "strategy": {
            "matrix": {
                "experimental": [False, True],
                "exclude": [{"EXPERIMENTAL": False}],
            }
        },
    }
    assert not job_tolerance_is_acceptable(job), (
        "an exclusion GitHub honours must remove the required leg here too"
    )
