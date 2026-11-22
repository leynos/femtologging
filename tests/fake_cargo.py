"""A recording `cargo` stand-in for the environment-policy contract.

The contract in `tests/test_env_access_policy_contract.py` judges status
propagation and argument order through the real Make → uv → lane-driver
boundary. Cargo itself is replaced: the accepting `none` lane once waited 420
seconds on Cargo's package-cache lock before reporting anything ([#537]), and
that wait measures another job's build rather than this repository's policy.
The stand-in records each invocation, writes a standard-error diagnostic naming
the lane it serves, fails only the lane a test names, and records an
allow-listed slice of the environment it ran with.

[#537]: https://github.com/leynos/femtologging/issues/537
"""

from __future__ import annotations

import json
import shlex
import sys
import typing as typ

from tests.make_contract_helpers import environment_overridable_variables

if typ.TYPE_CHECKING:
    from pathlib import Path

#: GNU Make's control variables. `MAKEFLAGS` carries command-line flags such as
#: `-i`, which would silence a rejected lane's failure, and both it and
#: `MAKEFILES` can introduce variable assignments the Makefile parse cannot
#: see. A measured run must therefore start without any of them.
MAKE_CONTROL_VARIABLES: typ.Final = ("MAKEFLAGS", "GNUMAKEFLAGS", "MAKEFILES")

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


def _observed_environment_names() -> tuple[str, ...]:
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
        *MAKE_CONTROL_VARIABLES,
    )


def _fake_cargo_runner(observed_names: tuple[str, ...]) -> str:
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
# tests/fake_cargo.py for the contract it serves.
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

    This only writes the launcher and names the paths it will use. It neither
    changes this process's `PATH` nor creates a log: a run records itself when
    something invokes it. `run_policy_target` puts the launcher on the child's
    `PATH`, and the stand-in appends to both logs as it runs.

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


def recorded_invocations(log: Path) -> tuple[tuple[str, ...], ...]:
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
