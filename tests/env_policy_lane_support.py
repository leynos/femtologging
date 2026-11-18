"""Run the environment-policy Make target against the recording Cargo.

The contract in `tests/test_env_access_policy_contract.py` judges status
propagation and exact argument order through the real Make → uv → lane-driver
boundary. This module builds that run: it takes the Makefile's defaults from
the parsed Makefile, describes the argument vector each lane implies, clears
the inherited variables that would change the outcome, and starts the target
with the stand-in from `tests/fake_cargo.py` first on the child's `PATH`.
"""

from __future__ import annotations

import os
import pathlib
import shutil
import signal
import subprocess  # ruff: ignore[suspicious-subprocess-import] contract tests run the real target
import typing as typ

from tests.fake_cargo import (
    FAKE_CARGO_DIAGNOSTIC_PREFIX,
    MAKE_CONTROL_VARIABLES,
    FakeCargo,
)
from tests.make_contract_helpers import (
    environment_overridable_variables,
    repo_root,
    sole_variable,
    variable_tokens,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

type Tokens = tuple[str, ...]

#: How long one `make lint-env-policy` invocation may run. Fake Cargo compiles
#: nothing, so this budget covers provisioning the lane driver through uv and
#: the real Make run; see issue #537 and the debugging plan of 2026-09-08.
MAKE_TIMEOUT_SECONDS: typ.Final = 120

#: Outer pytest bound for one `make lint-env-policy` run. Each contract test
#: starts one subprocess, so this sits above `MAKE_TIMEOUT_SECONDS` with room
#: for fixture setup. Fake Cargo compiles nothing, so the budget covers uv
#: provisioning and the real Make run, not a compiler queue or Cargo's
#: package-cache lock (issue #537). It lives here so every module that runs the
#: target is bounded the same way.
CONTRACT_TIMEOUT_SECONDS: typ.Final = MAKE_TIMEOUT_SECONDS + 60


def lane_selection(lane: str) -> Tokens:
    """Return the feature flags that select `lane`.

    The mapping is spelled out here rather than imported from the driver, so
    the contract does not confirm the driver through its own implementation.

    Returns
    -------
    tuple[str, ...]
        The flags Cargo receives for the lane.
    """
    if lane == "none":
        return ("--no-default-features",)
    if lane == "all":
        return ("--all-features",)
    return ("--no-default-features", "--features", lane)


def expected_invocation(
    manifest: str, lane: str, cargo_args: Tokens, lint_args: Tokens
) -> Tokens:
    """Return the exact argument vector the driver must give Cargo for `lane`.

    Returns
    -------
    tuple[str, ...]
        Every argument after the `cargo` executable name, in order.
    """
    # `--` separates Cargo's own flags from the arguments meant for Clippy, so
    # it exists only when there are Clippy arguments to separate.
    lint_separator: Tokens = ("--",) if lint_args else ()
    return (
        "clippy",
        "--manifest-path",
        manifest,
        *lane_selection(lane),
        *cargo_args,
        *lint_separator,
        *lint_args,
    )


def policy_defaults() -> tuple[str, Tokens, Tokens]:
    """Return the Makefile's default manifest and argument lists.

    Returns
    -------
    tuple[str, tuple[str, ...], tuple[str, ...]]
        The manifest path, Cargo arguments, and lint arguments.

    Raises
    ------
    TypeError
        If Makeutil does not report a string value for `RUST_MANIFEST`.
    """
    manifest = sole_variable("RUST_MANIFEST").get("raw_value")
    if not isinstance(manifest, str):
        message = "Makeutil must report RUST_MANIFEST as a string"
        raise TypeError(message)
    return (
        manifest,
        variable_tokens("ENV_POLICY_CARGO_ARGS"),
        variable_tokens("ENV_POLICY_LINT_ARGS"),
    )


def _search_path_of(environment: cabc.Mapping[str, str]) -> str:
    """Return the child's `PATH` with relative entries resolved for lookup.

    `Popen` starts the target from `repo_root()`, so a relative entry means that
    directory plus the entry. `shutil.which` would instead resolve it against
    whichever directory pytest was started from, so it could reject a Make the
    child would find, or find a different one. An empty entry means the working
    directory itself.

    Returns
    -------
    str
        The search path to hand to `shutil.which`.
    """
    root = repo_root()
    return os.pathsep.join(
        entry if pathlib.Path(entry).is_absolute() else str(root / entry)
        for entry in environment.get("PATH", "").split(os.pathsep)
    )


def run_policy_target(
    fake: FakeCargo,
    *overrides: str,
    reject: str | None = None,
    parent_environment: cabc.Mapping[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    """Run the real `make lint-env-policy` target with a fake Cargo.

    Make, uv and the lane driver run for real; only Cargo is replaced. That
    preserves status propagation and argument order while keeping the test off
    the shared package cache. Make variables not overridden keep their
    Makefile defaults, whatever this process exports.

    `parent_environment` replaces the process environment as the source of
    inherited variables, so a test can inject one without changing shared
    process state. The mapping is copied and never written to, and `make`
    itself is resolved against the `PATH` it supplies.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The target's exit status and captured output.

    Raises
    ------
    RuntimeError
        If `make` is not on the `PATH` of the environment being run.
    subprocess.TimeoutExpired
        If the target outlives `MAKE_TIMEOUT_SECONDS`; its process group is
        killed and reaped first, so no descendant keeps running against the
        fixture.
    KeyboardInterrupt
        If the caller is interrupted; the process group is killed and reaped
        before the interruption propagates, for the same reason.
    """
    environment = dict(os.environ if parent_environment is None else parent_environment)
    # A `?=` Make variable takes the environment's value in preference to the
    # Makefile's, and the child inherits this process's environment. Clearing
    # every such name keeps the run on the defaults the contract reads from the
    # Makefile: an exported `ENV_POLICY_FEATURE_LANES` would change which lanes
    # run, and an exported `ENV_POLICY_LINT_ARGS` would change what Clippy is
    # asked to do. Command-line overrides still apply after removal, because
    # make ranks them above both. `CARGO` is `?=` too, but no recipe here
    # expands `$(CARGO)`, so the fake wins on `PATH` regardless.
    for name in environment_overridable_variables():
        environment.pop(name, None)
    # Make's own controls are a different leak: `MAKEFLAGS=-i` would make the
    # target ignore a rejected lane's exit status, and both `MAKEFLAGS` and
    # `MAKEFILES` can carry variable assignments or include a file that
    # defines them, which the loop above cannot see.
    for name in MAKE_CONTROL_VARIABLES:
        environment.pop(name, None)
    # Resolve `make` against the environment the child will actually get, not
    # this process's `PATH`: an injected environment is the whole point.
    make = shutil.which("make", path=_search_path_of(environment))
    if make is None:
        message = "make must be on PATH to run the policy target"
        raise RuntimeError(message)
    environment["PATH"] = os.pathsep.join((
        str(fake.directory),
        environment.get("PATH", ""),
    ))
    environment["FAKE_CARGO_LOG"] = str(fake.log)
    environment["FAKE_CARGO_ENVIRONMENT_LOG"] = str(fake.environment_log)
    # The fake reads this one, so it is not a Make variable to strip. An
    # ambient rejection could fail a lane this run expects to accept, and the
    # caller's choice always wins; see ADR-0006 on inherited variables.
    environment.pop("FAKE_CARGO_REJECT", None)
    if reject is not None:
        environment["FAKE_CARGO_REJECT"] = reject
    # ruff: ignore[subprocess-without-shell-equals-true] the argument vector is
    # fixed here and the executable is resolved above
    with subprocess.Popen(
        (make, "lint-env-policy", *overrides),
        cwd=repo_root(),
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    ) as process:
        try:
            stdout, stderr = process.communicate(timeout=MAKE_TIMEOUT_SECONDS)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            # `make` starts uv and the lane driver as descendants. Killing only
            # the direct child would leave them running, still holding this
            # run's directory and log, so signal the whole group and reap it
            # before re-raising, whether the wait expired or the caller
            # interrupted the test.
            os.killpg(process.pid, signal.SIGKILL)
            process.communicate()
            raise
        returncode = process.returncode
    return subprocess.CompletedProcess(process.args, returncode, stdout, stderr)


def captured_output(completed: subprocess.CompletedProcess[str]) -> str:
    """Return both output streams, labelled for an assertion message."""
    return f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"


def lane_diagnostics(
    completed: subprocess.CompletedProcess[str], lanes: Tokens
) -> dict[str, int]:
    """Return how many times each lane's diagnostic reached the caller.

    Counting per lane rather than in total is what rules out a driver that
    forwards one lane's line twice and drops another's: the total would still
    match, but the lane that was dropped would report zero. The count reads
    standard error alone, because a diagnostic is meant to arrive there; a
    driver that diverted one to standard output would not satisfy the contract
    even though the text reached the caller somehow. A line must equal the
    sentinel exactly, so a driver that prefixes every diagnostic has changed
    the output rather than satisfied the count.

    Returns
    -------
    dict[str, int]
        One count per requested lane, keyed by lane name.
    """
    lines = completed.stderr.splitlines()
    return {
        lane: lines.count(f"{FAKE_CARGO_DIAGNOSTIC_PREFIX} {lane}") for lane in lanes
    }
