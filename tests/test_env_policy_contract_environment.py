"""Contracts for the environment the policy contract hands its child process.

The contract tests in `tests/test_env_access_policy_contract.py` run the real
`make lint-env-policy` target, so their subprocess inherits enough of this
process for `uv` and Make to work. What that inherited environment must not do
is change the run: an exported `ENV_POLICY_FEATURE_LANES` would replace the
Makefile's `?=` default, and an inherited `FAKE_CARGO_REJECT` would fail a lane
the test expects to accept. Either failure would read as a wiring defect rather
than as leaked environment. Each test here injects the environment it means to
test, so no shared process state changes.

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
    recorded_invocations,
    run_policy_target,
    write_fake_cargo,
)
from tests.make_contract_helpers import variable_tokens

if typ.TYPE_CHECKING:
    from pathlib import Path

#: The lane the inherited value names. It is the first lane the target runs, so
#: a value that survived into the child would fail the target immediately.
INHERITED_REJECTION: typ.Final = "none"

#: A single lane, exported in place of the Makefile's list. A run that linted
#: only this lane would record one invocation instead of one per default lane.
INHERITED_LANES: typ.Final = "none"


@pytest.fixture
def fake_cargo(tmp_path: Path) -> FakeCargo:
    """Install a recording `cargo` first on `PATH` and return its log.

    Returns
    -------
    FakeCargo
        The directory holding the fake executable and the log it writes.
    """
    return write_fake_cargo(tmp_path)


def inherited_environment(**additions: str) -> dict[str, str]:
    """Return this process's environment with `additions` layered over it.

    Returns
    -------
    dict[str, str]
        A fresh mapping, so a test can prove the helper left it unchanged.
    """
    return {**os.environ, **additions}


@pytest.mark.timeout(CONTRACT_TIMEOUT_SECONDS)
def test_accepting_lanes_ignore_an_inherited_rejection(fake_cargo: FakeCargo) -> None:
    """Scenario: the parent environment already names a lane to reject.

    Invariant: the helper's own choice wins, so a stray `FAKE_CARGO_REJECT`
    cannot make the accepting case fail. The inherited value names `none`, the
    first lane the target runs, so a value that reached the child would fail
    the target before any later lane started. The injected mapping is also
    left untouched, which is what lets a test hand over its own environment.
    """
    parent = inherited_environment(FAKE_CARGO_REJECT=INHERITED_REJECTION)
    passing_lane = run_policy_target(fake_cargo, parent_environment=parent)
    assert parent["FAKE_CARGO_REJECT"] == INHERITED_REJECTION, (
        "run_policy_target must copy the environment it is given"
    )
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
    parent = inherited_environment(ENV_POLICY_FEATURE_LANES=INHERITED_LANES)
    lanes = variable_tokens("ENV_POLICY_FEATURE_LANES")
    completed = run_policy_target(fake_cargo, parent_environment=parent)
    assert completed.returncode == 0, (
        "an inherited ENV_POLICY_FEATURE_LANES must not change the run\n"
        f"{captured_output(completed)}"
    )
    assert lane_diagnostics(completed, lanes) == dict.fromkeys(lanes, 1), (
        "the run must lint every default lane, not the inherited subset\n"
        f"{captured_output(completed)}"
    )
    assert len(recorded_invocations(fake_cargo.log)) == len(lanes), (
        "one Cargo invocation per default lane is what an unaltered run makes"
    )
