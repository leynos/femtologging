"""Run the heavy workflow's failure notification against a stub ``gh``.

The contract tests pin the notification job's structure, but they cannot show
that its shell actually comments on an existing issue or opens one when it is
absent. These tests execute the workflow's own ``run`` script with a stub
``gh`` on ``PATH``, so the create and comment paths are exercised separately
and nothing is written to GitHub.
"""

from __future__ import annotations

import json
import os
import stat
import subprocess  # ruff: ignore[suspicious-subprocess-import] the tests execute the checked-in workflow script
from pathlib import Path

from tests.heavy_workflow_document import (
    ISSUE_TITLE,
    NOTIFY_JOB,
    WORKFLOW,
    job,
    load_workflow,
    run_text,
    steps,
)

_REPOSITORY = "leynos/femtologging"
_RUN_ID = "43845886471"
_RUN_URL = f"https://github.com/{_REPOSITORY}/actions/runs/{_RUN_ID}"
_RUN_SHA = "283617f37d3878561e375d25eb32df2d83c57a5c"
_RUN_DATE = "2026-11-17T17:00:00Z"
_EXISTING_ISSUE = "42"
_GH_STUB = '''\
#!/usr/bin/env python3
"""Record each gh invocation and answer the notification's read queries."""

import json
import os
import sys
from pathlib import Path

arguments = sys.argv[1:]
with Path(os.environ["GH_STUB_LOG"]).open("a", encoding="utf-8") as handle:
    handle.write(json.dumps(arguments) + "\\n")

command = arguments[:2]
if command == ["run", "view"]:
    print(os.environ["GH_STUB_RUN_DATE"])
elif command == ["issue", "list"]:
    print(os.environ["GH_STUB_ISSUE"])
'''


def notification_script() -> str:
    """Return the shipped shell command of the notification job.

    Returns
    -------
    str
        The notification job's sole ``run`` script.

    Raises
    ------
    ValueError
        If the job does not have exactly one runnable step.
    """
    found_steps = steps(
        job(load_workflow(), NOTIFY_JOB),
        subject=f"{WORKFLOW} job {NOTIFY_JOB!r}",
    )
    scripts = [script for script in map(run_text, found_steps) if script]
    if len(scripts) != 1:
        message = f"{NOTIFY_JOB} must have exactly one runnable step"
        raise ValueError(message)
    return scripts[0]


def _write_stub(directory: Path) -> None:
    """Write the stub ``gh`` executable into ``directory``."""
    stub = directory / "gh"
    stub.write_text(_GH_STUB, encoding="utf-8")
    stub.chmod(stub.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def _notification_environment(tmp_path: Path, *, issue: str) -> dict[str, str]:
    """Return an explicit child environment for one notification run."""
    return {
        "PATH": f"{tmp_path / 'bin'}{os.pathsep}{os.defpath}",
        "HOME": str(tmp_path),
        "GH_TOKEN": "stub-token",
        "ISSUE_TITLE": ISSUE_TITLE,
        "RUN_URL": _RUN_URL,
        "RUN_SHA": _RUN_SHA,
        "GITHUB_RUN_ID": _RUN_ID,
        "GITHUB_REPOSITORY": _REPOSITORY,
        "GH_STUB_LOG": str(tmp_path / "gh.log"),
        "GH_STUB_ISSUE": issue,
        "GH_STUB_RUN_DATE": _RUN_DATE,
    }


def run_notification(tmp_path: Path, *, issue: str) -> list[list[str]]:
    """Run the shipped script and return the ``gh`` argument lists it made.

    Parameters
    ----------
    tmp_path : pathlib.Path
        A scratch directory for the stub, its log and the child's home.
    issue : str
        The issue number the stub reports as the title match, or ``""`` for
        none.

    Returns
    -------
    list[list[str]]
        One argument list per ``gh`` invocation, in order.
    """
    (tmp_path / "bin").mkdir(exist_ok=True)
    _write_stub(tmp_path / "bin")
    environment = _notification_environment(tmp_path, issue=issue)
    completed = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed argv runs the checked-in workflow script
        ["/bin/bash", "--noprofile", "--norc", "-c", notification_script()],
        capture_output=True,
        check=False,
        env=environment,
        text=True,
        timeout=30,
    )
    assert completed.returncode == 0, completed.stderr
    log = Path(environment["GH_STUB_LOG"])
    return [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()]


def _matching(recorded: list[list[str]], command: tuple[str, str]) -> list[list[str]]:
    """Return the invocations that ran ``command``."""
    return [arguments for arguments in recorded if tuple(arguments[:2]) == command]


def _argument(arguments: list[str], name: str) -> str:
    """Return the value that follows ``name`` in an argument list."""
    return arguments[arguments.index(name) + 1]


class TestNotificationBranches:
    """Exercise the create and comment branches of the notification shell."""

    @staticmethod
    def test_an_absent_issue_is_created(tmp_path: Path) -> None:
        """Open a tracking issue carrying the run details when none exists."""
        recorded = run_notification(tmp_path, issue="")
        creates = _matching(recorded, ("issue", "create"))
        assert len(creates) == 1, "an absent issue must be created exactly once"
        assert not _matching(recorded, ("issue", "comment")), (
            "an absent issue must not be commented on"
        )
        arguments = creates[0]
        assert _argument(arguments, "--repo") == _REPOSITORY
        assert _argument(arguments, "--title") == ISSUE_TITLE
        body = _argument(arguments, "--body")
        assert _RUN_URL in body, "the tracking issue must name the run URL"
        assert _RUN_SHA in body, "the tracking issue must name the commit"
        assert _RUN_DATE in body, "the tracking issue must name the run date"

    @staticmethod
    def test_an_existing_issue_is_commented_on(tmp_path: Path) -> None:
        """Comment on the existing tracking issue instead of opening another."""
        recorded = run_notification(tmp_path, issue=_EXISTING_ISSUE)
        comments = _matching(recorded, ("issue", "comment"))
        assert len(comments) == 1, "an existing issue must be commented on once"
        assert not _matching(recorded, ("issue", "create")), (
            "an existing issue must not be created again"
        )
        arguments = comments[0]
        assert arguments[2] == _EXISTING_ISSUE, "the found issue must be the target"
        assert _argument(arguments, "--repo") == _REPOSITORY
        body = _argument(arguments, "--body")
        assert _RUN_URL in body, "the comment must name the run URL"
        assert _RUN_SHA in body, "the comment must name the commit"
        assert _RUN_DATE in body, "the comment must name the run date"
