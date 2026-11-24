#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.13"
# dependencies = ["cyclopts>=4", "plumbum"]
# ///
"""Run the ignored Rust tests once per Cargo feature set.

The scheduled heavy suite has to prove the crate builds and passes with every
supported feature selection: none at all, ``python``, ``log-compat``, and both
compatibility bridges together. That is one ``cargo test`` invocation per
selection, and each invocation has to be able to fail the step. A shell ``for``
loop reports only the status of its last command, so a rejection in an earlier
selection is discarded unless every call carries a guard — a guard that is easy
to drop and whose absence leaves a lane that still looks green. Here the
failure path is the default, the run stops at the first failing selection, and
the log names it.

``none`` is the reserved name for ``--no-default-features`` with no
``--features`` flag, because an empty name cannot survive a whitespace-separated
environment list. Any other name is passed to Cargo verbatim, so
``python,log-compat`` reaches Cargo as Cargo spells it.

Parameters arrive as ``INPUT_``-prefixed environment variables, so the workflow
exports names rather than assembling an argument array.
"""

from __future__ import annotations

import dataclasses as dc
import os
import sys
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
#: The reserved feature-set name selecting no features at all.
NO_FEATURES = "none"
#: The harness selection the heavy suite exists to run.
DEFAULT_TEST_ARGS: typ.Final = ("--ignored",)


def feature_selection(feature_set: str) -> list[str]:
    """Return the Cargo feature flags the named selection adds.

    >>> feature_selection("none")
    []
    >>> feature_selection("python")
    ['--features', 'python']
    >>> feature_selection("python,log-compat")
    ['--features', 'python,log-compat']

    Returns
    -------
        The Cargo flags that select the feature set, empty for ``none``.
    """
    if feature_set == NO_FEATURES:
        return []
    return ["--features", feature_set]


def cargo_test_command(
    manifest: Path, feature_set: str, test_args: cabc.Sequence[str]
) -> list[str]:
    """Return the full ``cargo test`` argument vector for one feature set.

    Arguments after ``--`` reach the test harness; the separator is omitted
    when there are none, so Cargo never receives a bare trailing separator.

    >>> cargo_test_command(Path("C"), "none", ())[0]
    'test'
    >>> cargo_test_command(Path("C"), "python", ())[-2:]
    ['--features', 'python']
    >>> cargo_test_command(Path("C"), "none", ("--ignored",))[-2:]
    ['--', '--ignored']

    Returns
    -------
        The complete argument vector, excluding the ``cargo`` executable.
    """
    argv = [
        "test",
        "--manifest-path",
        str(manifest),
        "--no-default-features",
        *feature_selection(feature_set),
    ]
    if test_args:
        argv.append("--")
        argv.extend(test_args)
    return argv


@dc.dataclass(frozen=True, slots=True)
class FeatureTestPlan:
    """Everything one feature-matrix run needs, so the walk takes a plan."""

    manifest: Path
    feature_sets: tuple[str, ...]
    test_args: tuple[str, ...] = DEFAULT_TEST_ARGS


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


def run_feature_sets(plan: FeatureTestPlan, run: Runner = run_cargo) -> int:
    """Run the tests once per feature set, stopping at the first failure.

    The runner is a parameter so the ordering invariant can be exercised
    without spawning a process per feature set.

    Returns
    -------
        The failing feature set's exit code, or 0 when every set passes. An
        empty set list or an empty harness selection returns 2 rather than
        reporting a clean run that proved nothing.
    """
    if not plan.feature_sets:
        print(
            "no feature sets requested; refusing to report success",
            file=sys.stderr,
            flush=True,
        )
        return 2
    if not plan.test_args:
        print(
            "no harness arguments requested; refusing to run the ordinary suite "
            "in place of the ignored one",
            file=sys.stderr,
            flush=True,
        )
        return 2
    for feature_set in plan.feature_sets:
        print(f"# Cargo test feature set: {feature_set}", flush=True)
        retcode = run(cargo_test_command(plan.manifest, feature_set, plan.test_args))
        if retcode != 0:
            print(
                f"# Cargo test failed for feature set: {feature_set} (exit {retcode})",
                flush=True,
            )
            return retcode
    return 0


@app.default
def main(
    *,
    manifest: Path,
    feature_sets: list[str],
    test_args: list[str] | None = None,
) -> int:
    """Run the tests once per feature set, stopping at the first failure.

    ``test_args`` defaults to ``--ignored``, the selection the heavy suite
    exists to run: an omitted harness selection would run the ordinary tests
    instead and pass without touching the ignored suite.

    Returns
    -------
        The failing feature set's exit code, or 0 when every set passes.
    """
    return run_feature_sets(
        FeatureTestPlan(
            manifest=manifest,
            feature_sets=tuple(feature_sets),
            test_args=(
                tuple(test_args) if test_args is not None else DEFAULT_TEST_ARGS
            ),
        )
    )


if __name__ == "__main__":
    raise SystemExit(app())
