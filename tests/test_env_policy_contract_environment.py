"""Contracts for the environment the policy contract hands its child process.

The contract tests in `tests/test_env_access_policy_contract.py` run the real
`make lint-env-policy` target, so their subprocess inherits enough of this
process for `uv` and Make to work. What the fake `cargo` reads is decided by the
test, though: an inherited `FAKE_CARGO_REJECT` would fail a lane the test
expects to accept, and the failure would read as a wiring defect rather than as
leaked environment.

See [ADR-0006](../docs/adrs/adr-0006-environment-seam-taxonomy.md) on inherited
variables that could change a child's outcome, and the
[2026-09-08 debugging plan revision][plan] for why the contract uses a fake at
all.

[plan]: ../docs/debugging/debugging-plan-2026-09-08-env-policy-timeout.md
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.env_policy_lane_support import (
    CONTRACT_TIMEOUT_SECONDS,
    FakeCargo,
    captured_output,
    run_policy_target,
    write_fake_cargo,
)

if typ.TYPE_CHECKING:
    from pathlib import Path

#: The lane the inherited value names. It is the first lane the target runs, so
#: a value that survived into the child would fail the target immediately.
INHERITED_REJECTION: typ.Final = "none"


@pytest.fixture
def fake_cargo(tmp_path: Path) -> FakeCargo:
    """Install a recording `cargo` first on `PATH` and return its log.

    Returns
    -------
    FakeCargo
        The directory holding the fake executable and the log it writes.
    """
    return write_fake_cargo(tmp_path)


@pytest.mark.timeout(CONTRACT_TIMEOUT_SECONDS)
def test_accepting_lanes_ignore_an_inherited_rejection(
    fake_cargo: FakeCargo, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Scenario: the parent environment already names a lane to reject.

    Invariant: the helper's own choice wins, so a stray `FAKE_CARGO_REJECT`
    cannot make the accepting case fail. The inherited value names `none`, the
    first lane the target runs, so a value that reached the child would fail
    the target before any later lane started.
    """
    monkeypatch.setenv("FAKE_CARGO_REJECT", INHERITED_REJECTION)
    passing_lane = run_policy_target(fake_cargo)
    assert passing_lane.returncode == 0, (
        "an inherited FAKE_CARGO_REJECT must not fail a lane the run expects "
        f"to accept\n{captured_output(passing_lane)}"
    )
