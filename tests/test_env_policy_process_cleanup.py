"""Contracts for the policy target's process-group cleanup.

`run_policy_target` starts Make in its own session, and Make starts `uv` and the
lane driver beneath it. If a run times out or is interrupted, killing only the
direct child would leave those descendants running: they keep the fixture
directory and the captured pipes open, and the next run shares the machine with
a process it cannot see. Both tests here install a Make stand-in that records
its own and its descendant's process ID, force each failure in turn, and check
that the whole group is gone before the exception reaches the caller.

The stand-in never invokes Cargo, so neither test touches the package cache or
compiles anything; see the 2026-09-08 debugging plan for why that matters.
"""

from __future__ import annotations

import os
import subprocess  # ruff: ignore[suspicious-subprocess-import] the contract runs the real target
import time
import typing as typ

import pytest

from tests import env_policy_lane_support
from tests.env_policy_lane_support import run_policy_target
from tests.fake_cargo import write_fake_cargo

if typ.TYPE_CHECKING:
    from pathlib import Path

#: How long the assertions wait for a killed process to disappear. A killed
#: process is reaped by its new parent, which is not this process, so the group
#: can outlive the call by a scheduling tick.
_REAP_DEADLINE_SECONDS: typ.Final = 5.0

#: Make stand-in: record both process IDs, then wait so the caller has to end
#: it. `$$` is the session leader Popen created, which is also the group ID.
#: `dirname` is not used because this test puts only the stand-in's own
#: directory on the child's `PATH`; `${0%/*}` needs no external command.
_STUBBORN_MAKE: typ.Final = """#!/bin/sh
set -eu
here=${0%/*}
printf '%s\\n' "$$" > "$here/make.pid"
/bin/sh -c 'printf "%s\\n" "$$" > "$1"; while :; do /bin/sleep 0.2; done' \\
    sh "$here/child.pid" &
wait
"""


def install_stubborn_make(directory: Path) -> Path:
    """Write the Make stand-in under `directory` and return the search path.

    Returns
    -------
    Path
        The directory to put first on the child's `PATH`.
    """
    directory.mkdir()
    executable = directory / "make"
    executable.write_text(_STUBBORN_MAKE, encoding="utf-8")
    executable.chmod(0o755)
    return directory


def wait_for_pid(path: Path) -> int:
    """Return the process ID the stand-in has written to `path`.

    Returns
    -------
    int
        The recorded process ID.

    Raises
    ------
    AssertionError
        If a complete number does not appear inside `_REAP_DEADLINE_SECONDS`.
    """
    deadline = time.monotonic() + _REAP_DEADLINE_SECONDS
    while time.monotonic() < deadline:
        try:
            return int(path.read_text(encoding="utf-8").strip())
        except (FileNotFoundError, ValueError):
            # The stand-in may not have started, or may be halfway through the
            # write; either way there is nothing to read yet.
            time.sleep(0.01)
    message = f"the stand-in never wrote a process ID to {path.name}"
    raise AssertionError(message)


def assert_process_group_is_gone(group_id: int) -> None:
    """Assert no process remains in `group_id`, allowing for reaping delay.

    A killed process is reaped by its new parent, so the group can outlive the
    call by a scheduling tick; the deadline allows for that without letting a
    survivor pass.
    """
    deadline = time.monotonic() + _REAP_DEADLINE_SECONDS
    while True:
        try:
            os.killpg(group_id, 0)
        except ProcessLookupError:
            return
        if time.monotonic() > deadline:
            pytest.fail(
                f"process group {group_id} outlived the run; a timed-out or "
                "interrupted target must not leave descendants behind"
            )
        time.sleep(0.02)


@pytest.mark.timeout(60)
def test_a_timed_out_run_kills_and_reaps_the_process_group(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Scenario: the target outlives its budget.

    Invariant: `run_policy_target` kills the whole session and reaps it before
    raising, so neither Make nor the descendant it started survives the call.
    Without the group kill, the descendant would still hold Make's pipes, and
    the reap that follows the kill would wait on it rather than return.
    """
    make_directory = install_stubborn_make(tmp_path / "fake-path")
    fake = write_fake_cargo(tmp_path)
    # The stand-in waits forever, so the budget has to expire for the test to
    # reach the cleanup path at all.
    monkeypatch.setattr(env_policy_lane_support, "MAKE_TIMEOUT_SECONDS", 1)
    with pytest.raises(subprocess.TimeoutExpired) as caught:
        run_policy_target(fake, parent_environment={"PATH": str(make_directory)})
    assert caught.value.timeout == 1, (
        "the expired budget must be reported as the timeout that expired"
    )
    make_pid = wait_for_pid(make_directory / "make.pid")
    wait_for_pid(make_directory / "child.pid")
    assert_process_group_is_gone(make_pid)


@pytest.mark.timeout(60)
def test_an_interrupted_run_kills_and_reaps_the_process_group(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """Scenario: the caller interrupts the wait.

    Invariant: the interrupt propagates only after the session is killed and
    reaped, the same way a timeout does. A developer pressing Ctrl+C must not
    leave `uv` and the lane driver writing into a fixture directory that
    pytest deletes next. The wait is interrupted from inside the first
    `communicate` call, once both stand-in processes have recorded their IDs,
    so the interrupt cannot outrun the processes it is meant to clean up.
    """
    make_directory = install_stubborn_make(tmp_path / "fake-path")
    fake = write_fake_cargo(tmp_path)
    real_communicate = subprocess.Popen.communicate
    calls: list[int] = []

    def communicate_or_interrupt(
        process: subprocess.Popen[str],
        input_text: str | None = None,
        timeout: float | None = None,
    ) -> tuple[str, str]:
        """Interrupt the target's first wait, then defer to the real one."""
        # The helper parses the Makefile through `subprocess.run`, which also
        # reaches this method; only the target's own wait is interrupted. Its
        # argument vector is `(make, "lint-env-policy", ...)`; any other call
        # belongs to a different child and is left alone.
        target_args = process.args
        runs_the_target = (
            isinstance(target_args, (list, tuple))
            and len(target_args) > 1
            and target_args[1] == "lint-env-policy"
        )
        if not runs_the_target:
            return real_communicate(process, input_text, timeout)
        calls.append(process.pid)
        if len(calls) == 1:
            wait_for_pid(make_directory / "make.pid")
            wait_for_pid(make_directory / "child.pid")
            raise KeyboardInterrupt
        # The cleanup calls communicate a second time, after the group kill.
        return real_communicate(process, input_text, timeout)

    monkeypatch.setattr(subprocess.Popen, "communicate", communicate_or_interrupt)
    with pytest.raises(KeyboardInterrupt):
        run_policy_target(fake, parent_environment={"PATH": str(make_directory)})
    assert len(calls) == 2, (
        "the interrupted run must reap the group before the interrupt "
        f"propagates; communicate was called {len(calls)} time(s)"
    )
    assert_process_group_is_gone(wait_for_pid(make_directory / "make.pid"))
