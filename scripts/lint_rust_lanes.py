#!/usr/bin/env -S uv run python
# /// script
# requires-python = ">=3.13"
# dependencies = ["cyclopts>=4", "plumbum"]
# ///
"""Run Clippy over one Cargo feature lane at a time.

A lint that must hold under several feature selections needs one Cargo
invocation per selection, and every one of them has to be able to fail the
build. Expressing that as a shell ``for`` loop in the Makefile is what this
script replaces: a loop reports the status of its last command, so a rejection
in any earlier lane is discarded unless every call carries a guard. That guard
is easy to write and easy to drop, and dropping it turns a gate into a no-op
that still looks green. Here the failure path is the default and is covered by
tests.

Lane names map to feature selections:

``none``
    ``--no-default-features``
``all``
    ``--all-features``
anything else
    ``--no-default-features --features <name>``

``none`` and ``all`` between them compile both arms of every single-feature
gate, which ``--all-features`` alone cannot do because it never compiles a
``#[cfg(not(feature = ...))]`` block.

The script stops at the first lane that fails and names it, so the log ends at
the lane a contributor has to reproduce rather than at whichever lane happened
to run last.

Parameters arrive as ``INPUT_``-prefixed environment variables, so the Makefile
exports names rather than assembling an argument array.
"""

from __future__ import annotations

import dataclasses as dc
import os
import typing as typ

# Cyclopts resolves annotations at run time to coerce parameters, so `Path`
# must be a real import rather than a type-checking one.
from pathlib import Path  # ruff: ignore[typing-only-standard-library-import]

import cyclopts
from cyclopts import App
from plumbum import RETCODE, local

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    #: Runs one Cargo invocation and reports its exit code.
    type Runner = cabc.Callable[[cabc.Sequence[str]], int]

app = App(config=cyclopts.config.Env("INPUT_", command=False))

#: The Cargo executable, resolved from PATH at call time so a test harness can
#: shim it.
CARGO = "cargo"
#: Lane name selecting no features at all.
NO_FEATURES = "none"
#: Lane name selecting every declared feature.
ALL_FEATURES = "all"


def lane_selection(lane: str) -> list[str]:
    """Return the Cargo feature flags the named lane selects.

    >>> lane_selection("none")
    ['--no-default-features']
    >>> lane_selection("all")
    ['--all-features']
    >>> lane_selection("python")
    ['--no-default-features', '--features', 'python']

    Returns
    -------
        The Cargo flags that select the lane, in the order Cargo expects.
    """
    if lane == NO_FEATURES:
        return ["--no-default-features"]
    if lane == ALL_FEATURES:
        return ["--all-features"]
    return ["--no-default-features", "--features", lane]


def lane_command(
    manifest: Path,
    lane: str,
    cargo_args: cabc.Sequence[str],
    lint_args: cabc.Sequence[str],
) -> list[str]:
    """Return the full ``cargo clippy`` argument vector for one lane.

    Arguments after ``--`` reach the lint driver; they are omitted entirely
    when empty so that Cargo is never handed a bare trailing separator.

    Returns
    -------
        The complete argument vector, excluding the ``cargo`` executable.
    """
    argv = [
        "clippy",
        "--manifest-path",
        str(manifest),
        *lane_selection(lane),
        *cargo_args,
    ]
    if lint_args:
        argv.append("--")
        argv.extend(lint_args)
    return argv


@dc.dataclass(frozen=True, slots=True)
class LanePlan:
    """Everything one lint run needs, so the walk takes a plan and a runner."""

    manifest: Path
    lanes: tuple[str, ...]
    cargo_args: tuple[str, ...] = ()
    lint_args: tuple[str, ...] = ()


def run_cargo(argv: cabc.Sequence[str]) -> int:
    """Run Cargo with the given arguments, streaming its output.

    plumbum snapshots the process environment when it is imported, so a change
    made afterwards is invisible to the child: the executable is resolved
    against the stale PATH and later variables are not passed on. Re-exporting
    the live environment keeps the child's view equal to this script's, which
    is what lets a test harness shim `cargo` at all.

    Returns
    -------
        Cargo's exit code.
    """
    with local.env(**os.environ):
        return int(local[CARGO][list(argv)] & RETCODE(FG=True))


def lint_lanes(plan: LanePlan, run: Runner = run_cargo) -> int:
    """Lint every lane in turn, stopping at the first lane that fails.

    The runner is a parameter so the ordering invariant can be exercised
    without spawning a process per lane.

    Returns
    -------
        The failing lane's exit code, or 0 when every lane passes. An empty
        lane list returns non-zero rather than reporting a clean run it never
        made.
    """
    if not plan.lanes:
        print("no lanes requested; refusing to report success", flush=True)
        return 2
    for lane in plan.lanes:
        print(f"# Rust lint lane: {lane}", flush=True)
        argv = lane_command(plan.manifest, lane, plan.cargo_args, plan.lint_args)
        retcode = run(argv)
        if retcode != 0:
            print(f"# Rust lint lane failed: {lane} (exit {retcode})", flush=True)
            return retcode
    return 0


@app.default
def main(
    *,
    manifest: Path,
    lanes: list[str],
    cargo_args: list[str] | None = None,
    lint_args: list[str] | None = None,
) -> int:
    """Lint every lane in turn, stopping at the first lane that fails.

    Returns
    -------
        The failing lane's exit code, or 0 when every lane passes.
    """
    return lint_lanes(
        LanePlan(
            manifest=manifest,
            lanes=tuple(lanes),
            cargo_args=tuple(cargo_args or ()),
            lint_args=tuple(lint_args or ()),
        )
    )


if __name__ == "__main__":
    raise SystemExit(app())
