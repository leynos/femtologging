"""Contracts for the environment the policy contract hands its child process.

The contract tests in `tests/test_env_access_policy_contract.py` run the real
`make lint-env-policy` target, so their subprocess inherits enough of this
process for `uv` and Make to work. What that inherited environment must not do
is change the run: an exported `ENV_POLICY_FEATURE_LANES` would replace the
Makefile's `?=` default, `MAKEFLAGS=-i` would swallow a rejected lane's status,
and an inherited `FAKE_CARGO_REJECT` would fail a lane the test expects to
accept. Each failure would read as a wiring defect rather than as leaked
environment. Every test here injects the environment it means to test, so no
shared process state changes.

See [ADR-0006](../docs/adrs/adr-0006-environment-seam-taxonomy.md) on inherited
variables that could change a child's outcome, and the
[2026-09-08 debugging plan revision][plan] for why the contract uses a fake at
all.

[plan]: ../docs/debugging/debugging-plan-2026-09-08-env-policy-timeout.md
"""

from __future__ import annotations

import os
import typing as typ

import pytest

from tests.env_policy_lane_support import (
    CONTRACT_TIMEOUT_SECONDS,
    FakeCargo,
    captured_output,
    lane_diagnostics,
    recorded_child_environments,
    recorded_invocations,
    run_policy_target,
)
from tests.make_contract_helpers import variable_tokens

#: The lane the inherited value names. It is the first lane the target runs, so
#: a value that survived into the child would fail the target immediately.
INHERITED_REJECTION: typ.Final = "none"

#: A single lane, exported in place of the Makefile's list. A run that linted
#: only this lane would record one invocation instead of one per default lane.
INHERITED_LANES: typ.Final = "none"

#: A value only the injected parent can supply. Seeing it in the fake's record
#: is what proves the child received that mapping rather than this process's
#: environment, which is the difference an assertion on the injected mapping
#: alone cannot show.
INHERITED_MARKER: typ.Final = "marker-from-the-injected-parent"

#: The three lanes the failure-propagation contract runs, with the middle one
#: rejected. Kept short so a run that honours `MAKEFLAGS=-i` fails visibly.
FAILING_LANES: typ.Final = ("none", "python", "all")
REJECTED_LANE: typ.Final = "python"
FAILING_OVERRIDES: typ.Final = (
    f"ENV_POLICY_FEATURE_LANES={' '.join(FAILING_LANES)}",
    "ENV_POLICY_CARGO_ARGS=--lib --offline",
)


def inherited_environment(**additions: str) -> dict[str, str]:
    """Return this process's environment with `additions` layered over it.

    Returns
    -------
    dict[str, str]
        A fresh mapping, so a test can prove the helper left it unchanged.
    """
    return {**os.environ, **additions}


def assert_marker_reached_child(fake_cargo: FakeCargo) -> None:
    """Assert the child ran with the marker only the injected parent supplies.

    A missing record, or a record holding any other marker, means the injected
    mapping was not the one the child actually ran with.
    """
    received = recorded_child_environments(fake_cargo)
    assert received, "the fake must record the environment it ran with"
    markers = {seen.get("FAKE_CARGO_MARKER") for seen in received}
    assert markers == {INHERITED_MARKER}, (
        "every run must see the injected parent's marker, and only that one\n"
        f"recorded: {received!r}"
    )


@pytest.mark.timeout(CONTRACT_TIMEOUT_SECONDS)
def test_accepting_lanes_ignore_an_inherited_rejection(fake_cargo: FakeCargo) -> None:
    """Scenario: the parent environment already names a lane to reject.

    Invariant: the helper's own choice wins, so a stray `FAKE_CARGO_REJECT`
    cannot make the accepting case fail. The inherited value names `none`, the
    first lane the target runs, so a value that reached the child would fail
    the target before any later lane started. A marker alongside it has to
    reach the child, which shows the injected mapping was used at all, and the
    mapping is left untouched, which is what lets a test hand one over.
    """
    parent = inherited_environment(
        FAKE_CARGO_REJECT=INHERITED_REJECTION,
        FAKE_CARGO_MARKER=INHERITED_MARKER,
    )
    passing_lane = run_policy_target(fake_cargo, parent_environment=parent)
    assert parent["FAKE_CARGO_REJECT"] == INHERITED_REJECTION, (
        "run_policy_target must copy the environment it is given"
    )
    assert_marker_reached_child(fake_cargo)
    assert passing_lane.returncode == 0, (
        "an inherited FAKE_CARGO_REJECT must not fail a lane the run expects "
        f"to accept\n{captured_output(passing_lane)}"
    )


@pytest.mark.timeout(CONTRACT_TIMEOUT_SECONDS)
def test_inherited_make_variables_keep_the_makefile_defaults(
    fake_cargo: FakeCargo,
) -> None:
    """Scenario: the parent environment exports an `ENV_POLICY_` variable.

    Invariant: a `?=` variable the environment sets does not reach Make, so
    the run keeps the defaults the contract reads from the Makefile. Were the
    exported lane list to survive, the target would lint that one lane, which
    both narrows the policy check and contradicts the argument vectors the
    other contract tests expect.
    """
    parent = inherited_environment(
        ENV_POLICY_FEATURE_LANES=INHERITED_LANES,
        FAKE_CARGO_MARKER=INHERITED_MARKER,
    )
    lanes = variable_tokens("ENV_POLICY_FEATURE_LANES")
    completed = run_policy_target(fake_cargo, parent_environment=parent)
    assert completed.returncode == 0, (
        "an inherited ENV_POLICY_FEATURE_LANES must not change the run\n"
        f"{captured_output(completed)}"
    )
    received = recorded_child_environments(fake_cargo)
    assert_marker_reached_child(fake_cargo)
    assert {seen.get("ENV_POLICY_FEATURE_LANES") for seen in received} == {None}, (
        "the exported lane list must not reach the child"
    )
    assert lane_diagnostics(completed, lanes) == dict.fromkeys(lanes, 1), (
        "the run must lint every default lane, not the inherited subset\n"
        f"{captured_output(completed)}"
    )
    assert len(recorded_invocations(fake_cargo.log)) == len(lanes), (
        "one Cargo invocation per default lane is what an unaltered run makes"
    )


@pytest.mark.timeout(CONTRACT_TIMEOUT_SECONDS)
def test_inherited_makeflags_cannot_silence_a_rejected_lane(
    fake_cargo: FakeCargo,
) -> None:
    """Scenario: the parent environment exports `MAKEFLAGS=-i`.

    Invariant: Make's ignore-errors flag does not reach the child, so a
    rejected lane still fails the target. With the flag in place the target
    would exit 0 and the contract would report a passing run.
    """
    parent = inherited_environment(MAKEFLAGS="-i", FAKE_CARGO_MARKER=INHERITED_MARKER)
    failed_lane = run_policy_target(
        fake_cargo, *FAILING_OVERRIDES, reject=REJECTED_LANE, parent_environment=parent
    )
    # Make re-exports its own `MAKEFLAGS` to every recipe, so the child always
    # sees one; the exit status is what says the `-i` the parent supplied was
    # not carried into it. The marker shows the injected mapping was in force.
    assert_marker_reached_child(fake_cargo)
    assert failed_lane.returncode != 0, (
        "MAKEFLAGS=-i must not let a rejected lane pass\n"
        f"{captured_output(failed_lane)}"
    )
    verdicts = lane_diagnostics(failed_lane, FAILING_LANES)
    assert verdicts == {"none": 1, REJECTED_LANE: 1, "all": 0}, (
        f"the driver must stop after the rejected lane\n{captured_output(failed_lane)}"
    )
