"""Contract tests for a workflow ``run`` line the shell lexer cannot read.

The provisioning contract reads a step's shell text to find the suite it runs.
A line with an unclosed quote cannot be tokenised and may be hiding that very
command, so the reader reports it rather than answering "no suite here". These
tests pin that boundary: an unreadable line is a complaint on a Linux job,
beside any suite the step still declares, and the goal reader raises instead of
emptying the line.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.workflow_command_helpers import UnreadableShellLineError, make_goals
from tests.workflow_contract_helpers import (
    depends_on_preflight,
    provisioning_problems,
)

if typ.TYPE_CHECKING:
    from tests.workflow_document_helpers import Job

SETUP_RUST: typ.Final[str] = (
    "leynos/shared-actions/.github/actions/setup-rust@"
    "ff1dd759dfffc0db3459e30e833f52437ee62b57"
)
PREFLIGHT: typ.Final[str] = "check-build-tools"
# Synthetic prerequisites keep these cases independent of the repository's
# Makefile; only `test` has to reach the preflight for the suite to be gated.
SYNTHETIC_PREREQUISITES: typ.Final[dict[str, tuple[str, ...]]] = {
    "test": (PREFLIGHT,),
}
INSTALL_STEP: typ.Final[Job] = {
    "name": "Install rust",
    "uses": SETUP_RUST,
    "with": {"install-mold": "true"},
}


def _job(*steps: Job, runs_on: object = "ubuntu-latest") -> Job:
    """Return a synthetic job holding `steps` on `runs_on`."""
    return {"runs-on": runs_on, "steps": list(steps)}


def _problems(*steps: Job, runs_on: object = "ubuntu-latest") -> list[str]:
    """Return the provisioning complaints for a job that holds `steps`."""
    return provisioning_problems(
        _job(*steps, runs_on=runs_on),
        depends_on_preflight(SYNTHETIC_PREREQUISITES),
    )


@pytest.mark.parametrize(
    "run",
    [
        "make test && echo 'unterminated",
        "cargo test --locked && printf 'oops",
    ],
    ids=["after-make", "after-cargo"],
)
def test_an_unreadable_run_line_cannot_pass_as_no_suite(run: str) -> None:
    """An unclosed quote is reported, not read as naming no goal.

    The install step is present, so a reader that emptied the line would find
    no suite and report nothing at all.
    """
    assert _problems(INSTALL_STEP, {"name": "Test", "run": run}) == [
        f"unparseable run line: {run}"
    ]


def test_an_unreadable_line_is_reported_beside_a_readable_suite() -> None:
    """A step that also runs the suite still reports the line it cannot read."""
    problems = _problems(
        INSTALL_STEP,
        {"name": "Test", "run": "make test\necho 'unterminated"},
    )
    assert problems == ["unparseable run line: echo 'unterminated"], problems


def test_a_non_linux_job_is_out_of_scope_even_when_unreadable() -> None:
    """The contract governs Linux jobs, so another runner is not reported."""
    problems = _problems(
        {"name": "Test", "run": "make test && echo 'unterminated"},
        runs_on="windows-2022",
    )
    assert problems == [], problems


def test_make_goals_refuses_a_line_it_cannot_tokenise() -> None:
    """The reader raises instead of returning an empty goal list."""
    with pytest.raises(UnreadableShellLineError, match="unparseable shell line"):
        make_goals("make test && echo 'unterminated")
