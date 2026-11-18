"""Contracts for the tracked Python lockfile policy.

The repository commits ``uv.lock`` so development, CI and package builds
resolve the same dependency graph. The policy is enforced at three points: the
Makefile's ``check-lock`` gate, a dedicated CI step, and the heavy workflow's
locked install. These checks read repository files only, because the coverage
action and archive builds run outside a Git checkout.
"""

from __future__ import annotations

import re
import typing as typ

from tests.make_contract_helpers import (
    recipe_tokens,
    repo_root,
    sole_workflow_step,
)
from tests.makeutil_contract import _executable_lines
from tests.test_runner_placement_contract import Document, load_documents

LOCKFILE: typ.Final = "uv.lock"
CHECK_LOCK_STEP: typ.Final = "Check uv lock"
SYNC_STEP: typ.Final = "Install locked test dependencies"


def _commands(target: str) -> tuple[str, ...]:
    """Return the parsed `target` recipes as whitespace-joined commands.

    Parameters
    ----------
    target
        The Makefile target whose recipes to read.

    Returns
    -------
    tuple[str, ...]
        One command string per non-empty recipe.
    """
    return tuple(" ".join(tokens) for tokens in recipe_tokens(target) if tokens)


def _step_scripts(documents: dict[str, Document], workflow: str, job: str) -> list[str]:
    """Return every `run` script in one workflow job.

    Parameters
    ----------
    documents
        Parsed workflow documents keyed by file name.
    workflow
        The workflow file name.
    job
        The job name.

    Returns
    -------
    list[str]
        The step scripts in order.
    """
    steps = documents[workflow]["jobs"][job]["steps"]
    return [
        str(step.get("run", ""))
        for step in steps
        if isinstance(step, dict) and "run" in step
    ]


def test_the_lockfile_is_committed_at_the_repository_root() -> None:
    """The lock file the policy names exists in the checkout."""
    assert (repo_root() / LOCKFILE).is_file(), (
        f"{LOCKFILE} must be committed at the repository root"
    )


def test_gitignore_does_not_exclude_the_lockfile() -> None:
    """No active ignore pattern names the tracked lock file."""
    ignore_text = (repo_root() / ".gitignore").read_text(encoding="utf-8")
    active = [
        line.strip()
        for line in ignore_text.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    assert LOCKFILE not in active, f".gitignore must not ignore {LOCKFILE}"
    assert f"/{LOCKFILE}" not in active, f".gitignore must not ignore /{LOCKFILE}"


def test_check_lock_checks_freshness_tracking_and_ignore_rules() -> None:
    """The gate runs the lock check first, then proves tracking and visibility."""
    commands = _commands("check-lock")
    assert commands[0] == "uv lock --check", (
        f"check-lock must begin with the freshness check; got {commands!r}"
    )
    assert any(
        f"git ls-files --error-unmatch {LOCKFILE}" in line for line in commands
    ), "check-lock must require Git to track the lock file"
    assert any("git check-ignore --no-index" in line for line in commands), (
        "check-lock must prove no ignore rule hides the lock file"
    )


def test_ci_runs_the_lock_gate_once_in_build_test() -> None:
    """CI runs `make check-lock` exactly once, in the build-test job."""
    steps = load_documents()["ci.yml"]["jobs"]["build-test"]["steps"]
    matches = [
        step
        for step in steps
        if isinstance(step, dict)
        and str(step.get("run", "")).strip() == "make check-lock"
    ]
    assert len(matches) == 1, f"expected one lock check step, found {len(matches)}"
    assert matches[0].get("name") == CHECK_LOCK_STEP, (
        f"the lock check step must be named {CHECK_LOCK_STEP!r}"
    )


def test_heavy_workflow_builds_the_project_with_the_test_util_config() -> None:
    """Heavy CI's uv project builds carry the feature its tests assert."""
    job = load_documents()["heavy-tests.yml"]["jobs"]["heavy"]
    env = job.get("env", {})
    config_file = str(env.get("UV_CONFIG_FILE", ""))
    assert config_file.endswith(".github/uv-test-util.toml"), (
        "heavy CI must select the uv config that builds with test-util; "
        f"got {config_file!r}"
    )


def test_heavy_workflow_syncs_from_the_lock_and_never_through_pip() -> None:
    """Heavy CI installs the project group in locked mode, with no pip fallback."""
    sync = sole_workflow_step(".github/workflows/heavy-tests.yml", "heavy", SYNC_STEP)
    assert "uv sync --locked" in str(sync.get("run", "")), (
        "the heavy workflow must install its dev group from the committed lock"
    )

    for script in _step_scripts(load_documents(), "heavy-tests.yml", "heavy"):
        for line in _executable_lines(script):
            assert not re.search(r"(?<![\w-])pip install\b", line), (
                f"heavy CI must not install through pip: {line}"
            )
            is_project_command = re.search(r"\buv (sync|run)\b", line) is not None
            is_isolated = "--script" in line or "--no-project" in line
            assert not (
                is_project_command and not is_isolated and "--locked" not in line
            ), f"heavy CI project commands must pass --locked: {line}"
