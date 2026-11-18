"""Run the environment-policy Make target with a recording Cargo stand-in.

The contract in `tests/test_env_access_policy_contract.py` judges status
propagation and exact argument order through the real Make → uv → lane-driver
boundary. Cargo itself is replaced: the accepting `none` lane once waited 420
seconds on Cargo's package-cache lock before reporting anything ([#537]), and
that wait measures another job's build rather than this repository's policy.
The fake records each invocation, writes a standard-error diagnostic naming the
lane it serves, and fails only the lane a test names.
"""

from __future__ import annotations

import json
import os
import shlex
import shutil
import signal
import subprocess  # ruff: ignore[suspicious-subprocess-import] contract tests run the real target
import sys
import typing as typ

from tests.make_contract_helpers import (
    environment_overridable_variables,
    repo_root,
    sole_variable,
    variable_tokens,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

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

#: GNU Make's control variables. `MAKEFLAGS` carries command-line flags such as
#: `-i`, which would silence a rejected lane's failure, and both it and
#: `MAKEFILES` can introduce variable assignments the Makefile parse cannot
#: see. A measured run must therefore start without any of them.
_MAKE_CONTROL_VARIABLES: typ.Final = ("MAKEFLAGS", "GNUMAKEFLAGS", "MAKEFILES")

#: The variable a test exports to prove its environment reached the child. Only
#: an injected parent can supply this value, so finding it in the stand-in's
#: record distinguishes the caller's mapping from this process's environment.
MARKER_VARIABLE: typ.Final = "FAKE_CARGO_MARKER"

#: The `cargo` executable. A shell launcher names the interpreter instead of a
#: `#!` line doing so, because a deep virtualenv path can outgrow the kernel's
#: shebang limit and the fake would then fail before recording anything. The
#: quoted interpreter path keeps this working when it contains spaces.
_FAKE_CARGO_LAUNCHER: typ.Final = f"""#!/bin/sh
exec {shlex.quote(sys.executable)} "$(dirname "$0")/cargo_runner.py" "$@"
"""

#: The prefix of a line every fake Cargo run writes to standard error. The
#: lane name follows it, so the contract can prove that *each* lane's
#: diagnostics reached the caller. A shared, lane-less sentence would be
#: satisfied by the same lane's line repeated, which does not show that the
#: driver forwards every lane.
FAKE_CARGO_DIAGNOSTIC_PREFIX: typ.Final = "fake cargo: policy lane diagnostic for"

#: The full diagnostic line, with `{}` standing in for the lane name. Spelled
#: out here so the generated stand-in can print it without a nested f-string.
_LANE_DIAGNOSTIC_FORMAT: typ.Final = f"{FAKE_CARGO_DIAGNOSTIC_PREFIX} {{}}"


def _observed_environment_names() -> Tokens:
    """Return the variable names the stand-in records, in a stable order.

    The record is an allow-list rather than a prefix filter: the `?=` names
    come from the Makefile, the marker and Make's controls are named here, and
    nothing else in the child's environment is copied to the fixture directory.

    Returns
    -------
    tuple[str, ...]
        The names to copy from the child's environment.
    """
    return (
        *environment_overridable_variables(),
        MARKER_VARIABLE,
        *_MAKE_CONTROL_VARIABLES,
    )


def _fake_cargo_runner(observed_names: Tokens) -> str:
    """Return the recording Cargo stand-in's source.

    It logs each argument vector, writes a `_LANE_DIAGNOSTIC_FORMAT` line naming
    the lane to standard error, derives the lane from the driver's own flags,
    rejects at most the lane named by `FAKE_CARGO_REJECT`, and records
    `observed_names`. Exit status 101 mirrors Cargo's failed compile so the
    driver has a real failure to propagate.

    Returns
    -------
    str
        The program source, with the allow-list embedded as a literal.
    """
    allow_list = json.dumps(list(observed_names))
    return f"""# Recording Cargo stand-in; see
# tests/env_policy_lane_support.py for the contract it serves.
import json
import os
import sys

OBSERVED_NAMES = {allow_list}

arguments = sys.argv[1:]
with open(os.environ["FAKE_CARGO_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps(arguments) + "\\n")

environment_log = os.environ.get("FAKE_CARGO_ENVIRONMENT_LOG")
if environment_log is not None:
    observed = {{
        name: os.environ[name] for name in OBSERVED_NAMES if name in os.environ
    }}
    with open(environment_log, "a", encoding="utf-8") as log:
        log.write(json.dumps(observed) + "\\n")

if "--all-features" in arguments:
    lane = "all"
elif "--features" in arguments:
    lane = arguments[arguments.index("--features") + 1]
else:
    lane = "none"

print({_LANE_DIAGNOSTIC_FORMAT!r}.format(lane), file=sys.stderr)

raise SystemExit(101 if lane == os.environ.get("FAKE_CARGO_REJECT") else 0)
"""


class FakeCargo(typ.NamedTuple):
    """A recording Cargo stand-in and the log it appends invocations to."""

    directory: Path
    log: Path

    @property
    def environment_log(self) -> Path:
        """The sidecar recording the environment each run received."""
        return self.log.with_name("cargo-environments.jsonl")


def write_fake_cargo(directory: Path) -> FakeCargo:
    """Install an executable `cargo` under `directory/bin` and return its log.

    Returns
    -------
    FakeCargo
        The directory holding the fake executable and the log it writes.
    """
    binary_directory = directory / "bin"
    binary_directory.mkdir()
    (binary_directory / "cargo_runner.py").write_text(
        _fake_cargo_runner(_observed_environment_names()), encoding="utf-8"
    )
    executable = binary_directory / "cargo"
    executable.write_text(_FAKE_CARGO_LAUNCHER, encoding="utf-8")
    executable.chmod(0o755)
    return FakeCargo(directory=binary_directory, log=directory / "cargo-calls.jsonl")


def recorded_invocations(log: Path) -> tuple[Tokens, ...]:
    """Return the Cargo argument vectors the fake recorded, in call order.

    Returns
    -------
    tuple[tuple[str, ...], ...]
        One argument vector per call, in the order the driver made them.
    """
    if not log.exists():
        return ()
    return tuple(
        tuple(typ.cast("list[str]", json.loads(line)))
        for line in log.read_text(encoding="utf-8").splitlines()
    )


def recorded_child_environments(fake: FakeCargo) -> tuple[dict[str, str], ...]:
    """Return the control variables each run received, in call order.

    Returns
    -------
    tuple[dict[str, str], ...]
        One mapping per invocation, holding the Makefile's `?=` names, the
        marker variable and Make's control variables that run saw. A name the
        run did not receive is absent, not empty.
    """
    if not fake.environment_log.exists():
        return ()
    return tuple(
        typ.cast("dict[str, str]", json.loads(line))
        for line in fake.environment_log.read_text(encoding="utf-8").splitlines()
    )


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
    for name in _MAKE_CONTROL_VARIABLES:
        environment.pop(name, None)
    # Resolve `make` against the environment the child will actually get, not
    # this process's `PATH`: an injected environment is the whole point.
    make = shutil.which("make", path=environment.get("PATH", ""))
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
    even though the text reached the caller somehow.

    Returns
    -------
    dict[str, int]
        One count per requested lane, keyed by lane name.
    """
    output = completed.stderr
    return {
        lane: output.count(f"{FAKE_CARGO_DIAGNOSTIC_PREFIX} {lane}\n") for lane in lanes
    }
