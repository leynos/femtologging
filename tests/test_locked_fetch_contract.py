"""Contract tests for the locked-dependency fetch that precedes every test entry point.

Concordat's ``nested-cargo-offline-tests`` rule (NC-008) asks that a nested Cargo build in a
test never resolve dependencies while holding the shared package-cache lock. The ``test``
recipe of the Makefile and the CI and heavy-test workflows therefore run
``cargo fetch --locked`` first. These tests parse those files and hold the command, its
manifest, its position before every step that runs tests, and its success gating, with
mutation cases for each way the fetch could be lost.
"""

from __future__ import annotations

import re
import typing as typ
from pathlib import Path

import pytest
import yaml

ROOT: typ.Final[Path] = Path(__file__).resolve().parents[1]
MAKE_FETCH: typ.Final[str] = "cargo fetch --locked --manifest-path $(RUST_MANIFEST)"
WORKFLOW_FETCH: typ.Final[str] = (
    "cargo fetch --locked --manifest-path rust_extension/Cargo.toml"
)
FETCH_STEP: typ.Final[str] = "Fetch locked dependencies"
TEST_COMMAND: typ.Final[re.Pattern[str]] = re.compile(r"\b(cargo\s+test|make\s+test)\b")
Step = dict[str, typ.Any]


def _recipe(makefile: str, target: str) -> list[str]:
    """Return the recipe lines of one Makefile target, without their leading tab.

    Parameters
    ----------
    makefile
        The Makefile text.
    target
        The target whose recipe to read.

    Returns
    -------
    list[str]
        The recipe lines in order; empty when the target is absent.
    """
    lines = makefile.splitlines()
    start = next(
        (at for at, line in enumerate(lines) if re.match(rf"{target}:(?!=)", line)),
        None,
    )
    if start is None:
        return []
    recipe: list[str] = []
    for line in lines[start + 1 :]:
        if line.startswith("\t"):
            recipe.append(line[1:])
        elif line.strip():
            break
    return recipe


def _makefile_problems(makefile: str) -> list[str]:
    """Return the complaints about the ``test`` recipe's fetch.

    Parameters
    ----------
    makefile
        The Makefile text.

    Returns
    -------
    list[str]
        One complaint per way the fetch is missing, misplaced or not fail-fast.
    """
    recipe = _recipe(makefile, "test")
    fetch_at = next((at for at, line in enumerate(recipe) if "cargo fetch" in line), None)
    if fetch_at is None:
        return ["the test recipe runs no cargo fetch"]
    problems = []
    line = recipe[fetch_at]
    if line != MAKE_FETCH:
        problems.append(f"the fetch must be exactly `{MAKE_FETCH}`, found `{line}`")
    tests_before = [
        at for at, text in enumerate(recipe) if TEST_COMMAND.search(text) and at < fetch_at
    ]
    if tests_before:
        problems.append("the fetch must come before every command that runs tests")
    return problems


def _workflow_problems(steps: list[Step]) -> list[str]:
    """Return the complaints about a job's fetch step.

    Parameters
    ----------
    steps
        The job's steps in order.

    Returns
    -------
    list[str]
        One complaint per way the fetch step is missing, misplaced or skippable.
    """
    fetch_at = next(
        (at for at, step in enumerate(steps) if step.get("name") == FETCH_STEP), None
    )
    if fetch_at is None:
        return [f"the job has no `{FETCH_STEP}` step"]
    step = steps[fetch_at]
    problems = []
    if str(step.get("run", "")).strip() != WORKFLOW_FETCH:
        problems.append(f"the fetch step must run exactly `{WORKFLOW_FETCH}`")
    if "if" in step or step.get("continue-on-error"):
        problems.append("the fetch step must run on every run and fail the job")
    for at, other in enumerate(steps):
        if at < fetch_at and TEST_COMMAND.search(str(other.get("run", ""))):
            problems.append(f"step {other.get('name')!r} runs tests before the fetch")
    return problems


def _job_steps(workflow: str, job: str) -> list[Step]:
    """Return one job's steps from a workflow file of the repository.

    Parameters
    ----------
    workflow
        Workflow file name under ``.github/workflows``.
    job
        The job id.

    Returns
    -------
    list[Step]
        The job's steps.
    """
    document = yaml.safe_load((ROOT / ".github/workflows" / workflow).read_text("utf-8"))
    return typ.cast("list[Step]", document["jobs"][job]["steps"])


def test_the_makefile_test_recipe_fetches_first() -> None:
    """The ``test`` recipe runs the locked fetch before any test command."""
    problems = _makefile_problems((ROOT / "Makefile").read_text("utf-8"))
    assert not problems, problems


@pytest.mark.parametrize(
    ("workflow", "job"), [("ci.yml", "build-test"), ("heavy-tests.yml", "heavy")]
)
def test_the_workflows_fetch_before_their_test_steps(workflow: str, job: str) -> None:
    """Each workflow job fetches in an unconditional step before its test steps."""
    problems = _workflow_problems(_job_steps(workflow, job))
    assert not problems, problems


@pytest.mark.parametrize(
    "mutated",
    [
        "test:\n\tcargo test --manifest-path $(RUST_MANIFEST)\n",
        "test:\n\tcargo fetch --manifest-path $(RUST_MANIFEST)\n",
        "test:\n\tcargo fetch --locked\n",
        "test:\n\tcargo test --manifest-path $(RUST_MANIFEST)\n\t" + MAKE_FETCH + "\n",
    ],
    ids=["no-fetch", "not-locked", "wrong-manifest", "after-the-tests"],
)
def test_a_makefile_without_the_fetch_in_place_is_reported(mutated: str) -> None:
    """A missing, unlocked, mis-pointed or late fetch is reported."""
    assert _makefile_problems(mutated)


@pytest.mark.parametrize(
    "steps",
    [
        [{"name": "Test", "run": "cargo test"}],
        [{"name": FETCH_STEP, "run": "cargo fetch"}],
        [{"name": FETCH_STEP, "run": "cargo fetch --locked"}],
        [{"name": FETCH_STEP, "if": "always()", "run": WORKFLOW_FETCH}],
        [{"name": FETCH_STEP, "continue-on-error": True, "run": WORKFLOW_FETCH}],
        [
            {"name": "Test", "run": "cargo test"},
            {"name": FETCH_STEP, "run": WORKFLOW_FETCH},
        ],
    ],
    ids=[
        "no-fetch-step",
        "not-locked",
        "wrong-manifest",
        "conditional",
        "non-failing",
        "after-the-tests",
    ],
)
def test_a_job_without_the_fetch_in_place_is_reported(steps: list[Step]) -> None:
    """A missing, unlocked, mis-pointed, skippable, non-failing or late fetch is reported."""
    assert _workflow_problems(steps)


def test_a_job_with_the_fetch_first_is_accepted() -> None:
    """The reader accepts the intended shape, so the checks above prove something."""
    steps: list[Step] = [
        {"name": FETCH_STEP, "run": WORKFLOW_FETCH},
        {"name": "Test", "run": "cargo test"},
    ]
    assert not _workflow_problems(steps)
