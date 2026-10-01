#!/usr/bin/env -S uv run python
# /// script
# requires-python = ">=3.13"
# dependencies = ["cyclopts>=4", "plumbum"]
# ///
"""Run the Loom models and require every expected model to pass.

A green ``cargo test`` is not evidence that any model ran. A run that forgot
``--cfg loom`` compiles every model out and passes with zero tests; a filter
that matches nothing passes the same way; ``--ignored`` in place of
``--include-ignored`` runs a subset and passes; and ``--no-run`` passes having
executed nothing at all. This script closes those holes by comparing what the
run reported against a committed list of model names, in both directions:

- every listed model must be reported as ``ok``;
- nothing may be reported as ``ok`` that the list does not name, so ordinary
  tests cannot stand in for models;
- and Cargo itself must exit zero.

An ignored model needs no rule of its own: it is not reported as ``ok``, so it
is already a missing model.

The Cargo command follows ``--`` so the workflow, not this script, states it.
The run ends with a summary of the verdict, the counts, the preemption bound
and ``RUSTFLAGS``, so the job log says what was checked as well as what ran.
"""

from __future__ import annotations

import os
import re
import sys
import typing as typ

# Cyclopts resolves annotations at run time to coerce parameters, so `Path`
# must be a real import rather than a type-checking one.
from pathlib import Path  # ruff: ignore[typing-only-standard-library-import]

from cyclopts import App, Parameter
from plumbum import local

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    #: Runs one command and reports its exit code and standard output.
    type Runner = cabc.Callable[[cabc.Sequence[str]], tuple[int, str]]

app = App()

#: One libtest line reporting a passed test: ``test <name> ... ok``.
PASSED_LINE: typ.Final = re.compile(r"^test (?P<name>\S+) \.\.\. ok$")


def passed_tests(output: str) -> frozenset[str]:
    r"""Collect the names a libtest run reported as passed.

    >>> sorted(passed_tests("test a ... ok\ntest b ... FAILED\ntest c ... ignored"))
    ['a']

    Returns
    -------
    frozenset[str]
        The names of the tests reported ``ok``.
    """
    return frozenset(
        match["name"]
        for line in output.splitlines()
        if (match := PASSED_LINE.match(line.rstrip()))
    )


def read_expected(path: Path) -> frozenset[str]:
    """Read the expected model names, skipping blank lines and ``#`` comments.

    Returns
    -------
    frozenset[str]
        The names a run must report as passed.
    """
    lines = (line.strip() for line in path.read_text(encoding="utf-8").splitlines())
    return frozenset(line for line in lines if line and not line.startswith("#"))


def problems(
    returncode: int, passed: frozenset[str], expected: frozenset[str]
) -> list[str]:
    """Explain every way a run fails to show that each expected model passed.

    >>> problems(0, frozenset({"a"}), frozenset({"a"}))
    []

    Returns
    -------
    list[str]
        One sentence per problem, empty when the run is acceptable.
    """
    found: list[str] = []
    if not expected:
        found.append("the expected-model list is empty, so no run could prove anything")
    if returncode != 0:
        found.append(f"cargo test exited {returncode}")
    if missing := sorted(expected - passed):
        found.append(f"expected models not reported as passed: {', '.join(missing)}")
    if unexpected := sorted(passed - expected):
        found.append(
            f"passed tests the model list does not name: {', '.join(unexpected)}"
        )
    return found


def summary(passed: frozenset[str], expected: frozenset[str], found: list[str]) -> str:
    """Summarize the run: the verdict, the counts, the bound and the flags.

    >>> print(summary(frozenset({"a"}), frozenset({"a"}), []).splitlines()[0])
    Loom models: passed

    Returns
    -------
    str
        The summary, one fact per line, ending in each problem found.
    """
    verdict = "failed" if found else "passed"
    lines = [
        f"Loom models: {verdict}",
        f"  expected: {len(expected)}; passed: {len(passed & expected)}",
        f"  LOOM_MAX_PREEMPTIONS: {os.environ.get('LOOM_MAX_PREEMPTIONS', 'unset')}",
        f"  RUSTFLAGS: {os.environ.get('RUSTFLAGS', 'unset')}",
        *(f"  problem: {problem}" for problem in found),
    ]
    return "\n".join(lines)


def run_command(command: cabc.Sequence[str]) -> tuple[int, str]:
    """Run ``command``, echoing its output, and return its status and stdout.

    plumbum snapshots the process environment when it is imported, so the live
    environment is re-exported for the child, as in ``lint_rust_lanes.py``.

    Returns
    -------
    tuple[int, str]
        The exit code and the captured standard output.
    """
    program, *arguments = command
    with local.env(**os.environ):
        returncode, stdout, stderr = local[program][arguments].run(retcode=None)
    sys.stdout.write(stdout)
    sys.stderr.write(stderr)
    return int(returncode), str(stdout)


def check_models(
    command: cabc.Sequence[str], expected: Path, run: Runner = run_command
) -> int:
    """Run ``command`` and check it reported every expected model as passed.

    The runner is a parameter so the checks can be exercised without Cargo.

    Returns
    -------
    int
        0 when every expected model passed and nothing else did, 1 when not,
        and 2 when there is no command to run.
    """
    if not command:
        print("run_loom_models: no cargo command given after --", file=sys.stderr)
        return 2
    returncode, stdout = run(command)
    passed = passed_tests(stdout)
    wanted = read_expected(expected)
    found = problems(returncode, passed, wanted)
    print(summary(passed, wanted, found), file=sys.stderr)
    return 1 if found else 0


@app.default
def main(
    *command: str,
    expected: typ.Annotated[Path, Parameter(name="--expected")],
) -> int:
    """Run the Cargo command after ``--`` and check the models it reported.

    Returns
    -------
    int
        The result of :func:`check_models`.
    """
    return check_models(command, expected)


if __name__ == "__main__":
    raise SystemExit(app())
