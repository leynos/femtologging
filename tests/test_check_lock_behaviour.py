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

import shlex
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the tests run fixed recipe commands
import typing as typ
from pathlib import Path

import pytest

from tests.check_lock_recipe_helpers import (
    CHECK_LOCK_TARGET,
    IGNORED_MESSAGE,
    MUTANTS,
    recipe_commands,
    recipe_problems,
)

_LOCKFILE: typ.Final = "uv.lock"
_UV_STUB: typ.Final = "#!/bin/sh\nexit 0\n"
_UV_FAILING_STUB: typ.Final = "#!/bin/sh\nexit 1\n"

if typ.TYPE_CHECKING:
    import collections.abc as cabc


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
        [GIT, *arguments],
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


def _stub_tools(
    root: Path,
    *,
    uv_exit_code: int,
    git_script: str | None,
) -> str:
    """Write the stub tools and return the restricted `PATH` that finds them.

    The recipe runs `git` by name, so the resolved executable's directory must
    stay reachable even when Git lives outside ``/usr/bin``.

    Parameters
    ----------
    root
        The temporary repository that owns the stub directory.
    uv_exit_code
        The exit status the stubbed ``uv`` should report.
    git_script
        A stub ``git`` to install, or ``None`` to leave the real one in place.

    Returns
    -------
    str
        The ``PATH`` value for the recipe's commands.
    """
    stub_dir = root / "stub-bin"
    stub_dir.mkdir(exist_ok=True)
    stub = stub_dir / "uv"
    stub.write_text(
        _UV_STUB if uv_exit_code == 0 else _UV_FAILING_STUB,
        encoding="utf-8",
    )
    stub.chmod(0o755)
    if git_script is not None:
        git_stub = stub_dir / "git"
        git_stub.write_text(git_script, encoding="utf-8")
        git_stub.chmod(0o755)
    git_dir = str(Path(GIT).parent) if GIT else ""
    # The discovered directory must win over the conventional ones: a Git
    # found in /bin would otherwise lose to /usr/bin in the recipe's lookup.
    search_path = [str(stub_dir)]
    if git_dir:
        search_path.append(git_dir)
    search_path.extend(path for path in ("/usr/bin", "/bin") if path != git_dir)
    return ":".join(search_path)


def _run_commands(
    root: Path,
    commands: list[str],
    *,
    uv_exit_code: int = 0,
    git_script: str | None = None,
) -> subprocess.CompletedProcess[str]:
    """Run `commands` in `root` as make would, with stubbed tools on `PATH`.

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
    git_script
        A stub ``git`` to put first on ``PATH``, or ``None`` for the real one.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The last command's result, or the first failing one's.
    """
    environment = {
        "PATH": _stub_tools(root, uv_exit_code=uv_exit_code, git_script=git_script),
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


GIT: typ.Final = shutil.which("git") or ""
_NEEDS_GIT: typ.Final = pytest.mark.skipif(not GIT, reason="git is required")


@_NEEDS_GIT
def test_a_tracked_unignored_lock_passes(tmp_path: Path) -> None:
    """A fresh, tracked and visible lock satisfies every branch of the gate."""
    _write_repository(tmp_path, ignore=False, tracked=True)

    completed = _run_commands(tmp_path, recipe_commands(CHECK_LOCK_TARGET))

    assert completed.returncode == 0, (
        f"check-lock rejected a valid lock: {completed.stderr}"
    )
    assert IGNORED_MESSAGE not in completed.stderr, (
        f"a valid lock took the ignore branch: {completed.stderr}"
    )


@_NEEDS_GIT
def test_an_untracked_lock_fails(tmp_path: Path) -> None:
    """Git must track the lock, so an untracked file fails the gate."""
    _write_repository(tmp_path, ignore=False, tracked=False)

    completed = _run_commands(tmp_path, recipe_commands(CHECK_LOCK_TARGET))

    assert completed.returncode != 0, "an untracked lock passed the gate"
    assert "did not match any file" in completed.stderr, (
        f"the gate failed for a reason other than tracking: {completed.stderr}"
    )


@_NEEDS_GIT
def test_an_ignored_lock_fails_with_the_explanation(tmp_path: Path) -> None:
    """An ignore rule hides the lock, so the gate names the rule to remove."""
    _write_repository(tmp_path, ignore=True, tracked=True)

    completed = _run_commands(tmp_path, recipe_commands(CHECK_LOCK_TARGET))

    assert completed.returncode == 1, (
        f"an ignored lock must exit 1, got {completed.returncode}"
    )
    assert IGNORED_MESSAGE in completed.stderr, (
        f"the gate must name the ignore rule: {completed.stderr}"
    )


@_NEEDS_GIT
def test_a_failing_lock_check_stops_the_gate(tmp_path: Path) -> None:
    """A stale lock is refused by `uv lock --check`, before the Git checks run."""
    _write_repository(tmp_path, ignore=False, tracked=True)

    completed = _run_commands(
        tmp_path,
        recipe_commands(CHECK_LOCK_TARGET),
        uv_exit_code=1,
    )

    assert completed.returncode != 0, "a failing uv lock --check passed the gate"


def test_the_real_recipe_branches_on_every_probe_status() -> None:
    """The shipped recipe judges a match, a miss, and a probe error apart."""
    problems = recipe_problems(recipe_commands(CHECK_LOCK_TARGET))

    assert not problems, f"the shipped check-lock recipe is unsound: {problems}"


@pytest.mark.parametrize(
    ("mutate", "mutant"),
    [pytest.param(mutate, name, id=name) for mutate, name in MUTANTS],
)
def test_each_mutant_draws_a_complaint(
    mutate: cabc.Callable[[list[str]], list[str]],
    mutant: str,
) -> None:
    """A recipe that weakens any branch of the gate must be rejected."""
    original = recipe_commands(CHECK_LOCK_TARGET)
    mutated = mutate(original)

    assert mutated != original, (
        f"mutant {mutant} changed nothing, so it cannot detect the defect"
    )
    assert recipe_problems(mutated), f"mutant {mutant} was not detected"


@_NEEDS_GIT
def test_an_unexpected_probe_error_is_propagated(tmp_path: Path) -> None:
    """A probe failure other than 'not ignored' keeps its own status and message."""
    _write_repository(tmp_path, ignore=False, tracked=True)
    git_stub = (
        "#!/bin/sh\n"
        'case "$1" in\n'
        "  check-ignore) exit 2 ;;\n"
        f'  *) exec {shlex.quote(GIT)} "$@" ;;\n'
        "esac\n"
    )

    completed = _run_commands(
        tmp_path,
        recipe_commands(CHECK_LOCK_TARGET),
        git_script=git_stub,
    )

    assert completed.returncode == 2, (
        f"an unexpected probe status must propagate, got {completed.returncode}"
    )
    assert "git check-ignore failed with status 2" in completed.stderr, (
        f"the gate must name the probe failure: {completed.stderr}"
    )


@_NEEDS_GIT
def test_git_does_not_run_after_a_failed_lock_check(tmp_path: Path) -> None:
    """A stale lock stops the recipe before any Git command runs."""
    _write_repository(tmp_path, ignore=False, tracked=True)
    marker = tmp_path / "git-ran"
    git_stub = f"#!/bin/sh\ntouch {shlex.quote(str(marker))}\nexit 0\n"

    completed = _run_commands(
        tmp_path,
        recipe_commands(CHECK_LOCK_TARGET),
        uv_exit_code=1,
        git_script=git_stub,
    )

    assert completed.returncode != 0, "a failing uv lock --check passed the gate"
    assert not marker.exists(), "Git ran even though uv lock --check failed"
