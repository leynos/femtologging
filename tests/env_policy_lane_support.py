"""Run the environment-policy Make target with a recording Cargo stand-in.

The contract in `tests/test_env_access_policy_contract.py` judges status
propagation and exact argument order through the real Make → uv → lane-driver
boundary. Cargo itself is replaced: the accepting `none` lane once waited 420
seconds on Cargo's package-cache lock before reporting anything ([#537]), and
that wait measures another job's build rather than this repository's policy.
The fake records each invocation and fails only the lane a test names.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] contract tests run the real target
import sys
import typing as typ

from tests.make_contract_helpers import repo_root, sole_variable, variable_tokens

if typ.TYPE_CHECKING:
    from pathlib import Path

type Tokens = tuple[str, ...]

#: How long one `make lint-env-policy` invocation may run. Fake Cargo compiles
#: nothing, so this budget covers provisioning the lane driver through uv and
#: the real Make run; see issue #537 and the debugging plan of 2026-09-08.
MAKE_TIMEOUT_SECONDS: typ.Final = 120

#: The fake Cargo. It records every argument vector as one JSON array per line,
#: derives the lane from the driver's own flags, and rejects at most the lane
#: named by `FAKE_CARGO_REJECT`. Exit status 101 mirrors Cargo's failed compile
#: so the driver has a real failure to propagate.
_FAKE_CARGO_SCRIPT: typ.Final = f"""#!{sys.executable}
import json
import os
import sys

arguments = sys.argv[1:]
with open(os.environ["FAKE_CARGO_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps(arguments) + "\\n")

if "--all-features" in arguments:
    lane = "all"
elif "--features" in arguments:
    lane = arguments[arguments.index("--features") + 1]
else:
    lane = "none"

raise SystemExit(101 if lane == os.environ.get("FAKE_CARGO_REJECT") else 0)
"""


class FakeCargo(typ.NamedTuple):
    """A recording Cargo stand-in and the log it appends invocations to."""

    directory: Path
    log: Path


def write_fake_cargo(directory: Path) -> FakeCargo:
    """Install an executable `cargo` under `directory/bin` and return its log.

    Returns
    -------
    FakeCargo
        The directory holding the fake executable and the log it writes.
    """
    binary_directory = directory / "bin"
    binary_directory.mkdir()
    executable = binary_directory / "cargo"
    executable.write_text(_FAKE_CARGO_SCRIPT, encoding="utf-8")
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
    return (
        "clippy",
        "--manifest-path",
        manifest,
        *lane_selection(lane),
        *cargo_args,
        "--",
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
    fake: FakeCargo, *overrides: str, reject: str | None = None
) -> subprocess.CompletedProcess[str]:
    """Run the real `make lint-env-policy` target with a fake Cargo.

    Make, uv and the lane driver run for real; only Cargo is replaced. That
    preserves status propagation and argument order while keeping the test off
    the shared package cache. Make variables not overridden keep their
    Makefile defaults.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The target's exit status and captured output.

    Raises
    ------
    RuntimeError
        If `make` is not on `PATH`.
    """
    make = shutil.which("make")
    if make is None:
        message = "make must be on PATH to run the policy target"
        raise RuntimeError(message)
    environment = dict(os.environ)
    environment["PATH"] = os.pathsep.join((
        str(fake.directory),
        environment.get("PATH", ""),
    ))
    environment["FAKE_CARGO_LOG"] = str(fake.log)
    if reject is not None:
        environment["FAKE_CARGO_REJECT"] = reject
    # ruff: ignore[subprocess-without-shell-equals-true] the argument vector is
    # fixed here and the executable is resolved above
    return subprocess.run(
        (make, "lint-env-policy", *overrides),
        cwd=repo_root(),
        env=environment,
        capture_output=True,
        text=True,
        check=False,
        timeout=MAKE_TIMEOUT_SECONDS,
    )


def captured_output(completed: subprocess.CompletedProcess[str]) -> str:
    """Return both output streams, labelled for an assertion message."""
    return f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
