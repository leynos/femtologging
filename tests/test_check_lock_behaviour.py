"""Behavioural tests for the ``check-lock`` Makefile target.

``check-lock`` is the gate that keeps ``uv.lock`` fresh, tracked and visible.
Reading its recipe only proves the text says the right thing, so these tests
run the recipe against a throwaway Git repository and assert the exit status
and message for each case: a tracked lock that is not ignored, an untracked
lock, and a lock hidden by an ignore rule. ``uv lock --check`` is stubbed with
a no-op ``uv`` on ``PATH``, so the tests cover this repository's status
branching rather than the resolver.

The recipe is read from the real Makefile, so a change to it is executed here
rather than described. A mutation test proves the harness can fail: a recipe
that masks a bad status must be detected.
"""

from __future__ import annotations

import re
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the tests run fixed recipe commands
import typing as typ

import pytest

from tests.make_contract_helpers import objects, sole_recipe_rule

_CHECK_LOCK_TARGET: typ.Final = "check-lock"
_LOCKFILE: typ.Final = "uv.lock"
_IGNORED_MESSAGE: typ.Final = "uv.lock is ignored"
_UV_STUB: typ.Final = "#!/bin/sh\nexit 0\n"
_UV_FAILING_STUB: typ.Final = "#!/bin/sh\nexit 1\n"
_MASKED_PROBE: typ.Final[re.Pattern[str]] = re.compile(r"check-ignore[^\n]*\|\|\s*true")

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path


def _recipe_commands(target: str) -> list[str]:
    """Return the Makefile recipe for `target` as shell commands.

    The recipe is taken as makeutil reports it, so quoting and line
    continuations survive: tokenising with ``shlex`` would drop the quotes
    around the diagnostic messages. Make runs one shell per logical recipe
    line and stops at the first failure, so each command is returned
    separately, with make's ``$$`` escaping resolved to a literal ``$`` and
    its leading ``@`` removed.

    Parameters
    ----------
    target
        The Makefile target whose recipe to read.

    Returns
    -------
    list[str]
        One shell command per recipe line, in order.
    """
    recipes = objects(
        sole_recipe_rule(target).get("recipes"), subject=f"{target} recipes"
    )
    commands = []
    for recipe in recipes:
        text = recipe.get("text")
        if not isinstance(text, str):
            continue
        line = text.replace("$$", "$").removeprefix("@")
        if line.strip() and not line.lstrip().startswith("#"):
            commands.append(line)
    return commands


def _git(root: Path, *arguments: str) -> None:
    """Run one fixed Git command in `root`.

    Parameters
    ----------
    root
        The temporary repository.
    arguments
        The Git subcommand and its fixed arguments.
    """
    subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed git invocation
        ["/usr/bin/git", *arguments],
        cwd=root,
        check=True,
        capture_output=True,
    )


def _write_repository(root: Path, *, ignore: bool, tracked: bool) -> None:
    """Create a Git repository with ``uv.lock`` in the requested state."""
    root.mkdir(parents=True, exist_ok=True)
    _git(root, "init", "--quiet")
    _git(root, "config", "user.email", "tests@example.invalid")
    _git(root, "config", "user.name", "Contract Tests")
    (root / _LOCKFILE).write_text("version = 1\n", encoding="utf-8")
    (root / ".gitignore").write_text(
        f"{_LOCKFILE}\n" if ignore else "*.pyc\n",
        encoding="utf-8",
    )
    if tracked:
        _git(root, "add", "--force", _LOCKFILE, ".gitignore")


def _run_commands(
    root: Path,
    commands: list[str],
    *,
    uv_exit_code: int = 0,
) -> subprocess.CompletedProcess[str]:
    """Run `commands` in `root` as make would, with a stubbed `uv` on `PATH`.

    Each command gets its own shell, and the first failure stops the run, so
    a later command cannot mask an earlier one.

    Parameters
    ----------
    root
        The temporary repository to run the commands in.
    commands
        The recipe commands, in order.
    uv_exit_code
        The exit status the stubbed ``uv`` should report.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The last command's result, or the first failing one's.
    """
    stub_dir = root / "stub-bin"
    stub_dir.mkdir(exist_ok=True)
    stub = stub_dir / "uv"
    stub.write_text(
        _UV_STUB if uv_exit_code == 0 else _UV_FAILING_STUB,
        encoding="utf-8",
    )
    stub.chmod(0o755)
    environment = {
        "PATH": f"{stub_dir}:/usr/bin:/bin",
        "HOME": str(root),
        "GIT_CONFIG_NOSYSTEM": "1",
    }
    stdout, stderr = [], []
    for command in commands:
        completed = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed recipe command
            ["/bin/sh", "-c", command],
            cwd=root,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        stdout.append(completed.stdout)
        stderr.append(completed.stderr)
        if completed.returncode != 0:
            return subprocess.CompletedProcess(
                args=command,
                returncode=completed.returncode,
                stdout="".join(stdout),
                stderr="".join(stderr),
            )
    return subprocess.CompletedProcess(
        args=commands,
        returncode=0,
        stdout="".join(stdout),
        stderr="".join(stderr),
    )


@pytest.mark.skipif(shutil.which("git") is None, reason="git is required")
def test_a_tracked_unignored_lock_passes(tmp_path: Path) -> None:
    """A fresh, tracked and visible lock satisfies every branch of the gate."""
    _write_repository(tmp_path, ignore=False, tracked=True)

    completed = _run_commands(tmp_path, _recipe_commands(_CHECK_LOCK_TARGET))

    assert completed.returncode == 0, (
        f"check-lock rejected a valid lock: {completed.stderr}"
    )
    assert _IGNORED_MESSAGE not in completed.stderr, (
        f"a valid lock took the ignore branch: {completed.stderr}"
    )


@pytest.mark.skipif(shutil.which("git") is None, reason="git is required")
def test_an_untracked_lock_fails(tmp_path: Path) -> None:
    """Git must track the lock, so an untracked file fails the gate."""
    _write_repository(tmp_path, ignore=False, tracked=False)

    completed = _run_commands(tmp_path, _recipe_commands(_CHECK_LOCK_TARGET))

    assert completed.returncode != 0, "an untracked lock passed the gate"
    assert "did not match any file" in completed.stderr, (
        f"the gate failed for a reason other than tracking: {completed.stderr}"
    )


@pytest.mark.skipif(shutil.which("git") is None, reason="git is required")
def test_an_ignored_lock_fails_with_the_explanation(tmp_path: Path) -> None:
    """An ignore rule hides the lock, so the gate names the rule to remove."""
    _write_repository(tmp_path, ignore=True, tracked=True)

    completed = _run_commands(tmp_path, _recipe_commands(_CHECK_LOCK_TARGET))

    assert completed.returncode == 1, (
        f"an ignored lock must exit 1, got {completed.returncode}"
    )
    assert _IGNORED_MESSAGE in completed.stderr, (
        f"the gate must name the ignore rule: {completed.stderr}"
    )


@pytest.mark.skipif(shutil.which("git") is None, reason="git is required")
def test_a_failing_lock_check_stops_the_gate(tmp_path: Path) -> None:
    """A stale lock is refused by `uv lock --check`, before the Git checks run."""
    _write_repository(tmp_path, ignore=False, tracked=True)

    completed = _run_commands(
        tmp_path,
        _recipe_commands(_CHECK_LOCK_TARGET),
        uv_exit_code=1,
    )

    assert completed.returncode != 0, "a failing uv lock --check passed the gate"


def _recipe_problems(commands: list[str]) -> list[str]:
    """Return one complaint per way the gate's status branching is unsound.

    The branching is judged rather than described: a mutant recipe must draw a
    complaint, so a check that reports nothing cannot pass by accident.

    Parameters
    ----------
    commands
        The recipe's shell commands, in order.

    Returns
    -------
    list[str]
        One complaint per requirement the recipe fails to impose.
    """
    script = "\n".join(commands)
    problems: list[str] = []
    if not re.search(r"git check-ignore --no-index -q uv\.lock", script):
        problems.append("the ignore probe must run quietly against uv.lock")
    if "status -eq 0" not in script:
        problems.append("a matching ignore rule must fail the gate")
    if "status -ne 1" not in script:
        problems.append("a probe error other than 'not ignored' must propagate")
    if _MASKED_PROBE.search(script):
        problems.append("the probe's status must not be masked")
    if "git ls-files --error-unmatch uv.lock" not in script:
        problems.append("the gate must require Git to track the lock")
    return problems


def test_the_real_recipe_branches_on_every_probe_status() -> None:
    """The shipped recipe judges a match, a miss, and a probe error apart."""
    problems = _recipe_problems(_recipe_commands(_CHECK_LOCK_TARGET))

    assert not problems, f"the shipped check-lock recipe is unsound: {problems}"


def _unquiet_probe(commands: list[str]) -> list[str]:
    """Drop the probe's quiet flag."""
    return [
        command.replace("check-ignore --no-index -q", "check-ignore --no-index")
        for command in commands
    ]


def _inverted_match_branch(commands: list[str]) -> list[str]:
    """Treat the 'ignored' status as success."""
    return [command.replace("status -eq 0", "status -eq 1") for command in commands]


def _dropped_error_branch(commands: list[str]) -> list[str]:
    """Remove the branch that propagates an unexpected status."""
    return [
        re.sub(r"if \[ \$status -ne 1 \]; then.*?\nfi", "", command, flags=re.DOTALL)
        for command in commands
    ]


def _masked_probe(commands: list[str]) -> list[str]:
    """Swallow the probe's exit status."""
    return [
        command.replace(
            "git check-ignore --no-index -q uv.lock; status=$?",
            "git check-ignore --no-index -q uv.lock || true; status=$?",
        )
        for command in commands
    ]


def _untracked_check_removed(commands: list[str]) -> list[str]:
    """Drop the requirement that Git tracks the lock."""
    return [
        command for command in commands if "git ls-files --error-unmatch" not in command
    ]


@pytest.mark.parametrize(
    "mutate",
    [
        pytest.param(_unquiet_probe, id="unquiet-probe"),
        pytest.param(_inverted_match_branch, id="inverted-match-branch"),
        pytest.param(_dropped_error_branch, id="dropped-error-branch"),
        pytest.param(_masked_probe, id="masked-probe"),
        pytest.param(_untracked_check_removed, id="untracked-check-removed"),
    ],
)
def test_each_mutant_draws_a_complaint(
    mutate: cabc.Callable[[list[str]], list[str]],
) -> None:
    """A recipe that weakens any branch of the gate must be rejected."""
    mutated = mutate(_recipe_commands(_CHECK_LOCK_TARGET))

    assert _recipe_problems(mutated), f"mutant {mutate.__name__} was not detected"


@pytest.mark.skipif(shutil.which("git") is None, reason="git is required")
def test_an_ignored_lock_never_reports_the_ignore_message_on_success(
    tmp_path: Path,
) -> None:
    """A tracked, visible lock must not take the ignore-rule branch."""
    _write_repository(tmp_path, ignore=False, tracked=True)

    completed = _run_commands(tmp_path, _recipe_commands(_CHECK_LOCK_TARGET))

    assert completed.returncode == 0, (
        f"a hidden lock failed the gate: {completed.stderr}"
    )
    assert _IGNORED_MESSAGE not in completed.stderr, (
        f"a visible lock took the ignore branch: {completed.stderr}"
    )
