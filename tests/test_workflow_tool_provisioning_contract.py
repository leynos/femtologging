"""Contract tests for provisioning build tools before a hosted suite runs.

A coverage build may exclude ``mold`` from its compiler flags and still run
tests that reach Make's build-tool preflight, so every applicable Linux job
that runs the suite has to install the pinned toolchain and linker first.
These tests derive the affected jobs from workflow and Make reachability, then
prove the reader notices each way the installation can be lost.
"""

from __future__ import annotations

import typing as typ

import pytest
import yaml.constructor

from tests.make_contract_helpers import prerequisite_map
from tests.workflow_contract_helpers import (
    depends_on_preflight,
    make_goals,
    provisioning_problems,
    suite_steps,
)
from tests.workflow_document_helpers import (
    Job,
    is_linux_job,
    load_workflow,
    runner_values,
    workflow_jobs,
)

if typ.TYPE_CHECKING:
    from pathlib import Path

SETUP_RUST: typ.Final[str] = (
    "leynos/shared-actions/.github/actions/setup-rust@"
    "ff1dd759dfffc0db3459e30e833f52437ee62b57"
)


def _suite_jobs() -> list[tuple[str, str, Job]]:
    """Return every workflow job that runs a preflight-gated suite."""
    depends = depends_on_preflight()
    return [
        (workflow, job_id, job)
        for workflow, job_id, job in workflow_jobs()
        if suite_steps(job, depends)
    ]


def test_every_hosted_suite_job_provisions_the_build_tools() -> None:
    """Each Linux job running the suite installs the tools before it starts."""
    depends = depends_on_preflight()
    suite_jobs = _suite_jobs()
    found = {(workflow, job_id) for workflow, job_id, _ in suite_jobs}
    assert {("ci.yml", "build-test"), ("heavy-tests.yml", "heavy")} <= found, (
        f"the reader missed a known suite job; it found {sorted(found)}"
    )
    problems = [
        f"{workflow} {job_id}: {problem}"
        for workflow, job_id, job in suite_jobs
        for problem in provisioning_problems(job, depends)
    ]
    assert not problems, problems


INSTALL_STEP: typ.Final[Job] = {
    "name": "Install rust",
    "uses": SETUP_RUST,
    "with": {"install-mold": "true"},
}
CARGO_TEST: typ.Final[Job] = {
    "name": "Test",
    "run": "cargo test --manifest-path rust_extension/Cargo.toml",
}
MAKE_TEST: typ.Final[Job] = {"name": "Test", "run": "make test"}


def _job(*steps: Job, runs_on: object = "ubuntu-latest") -> Job:
    """Return a synthetic job holding `steps` on `runs_on`."""
    return {"runs-on": runs_on, "steps": list(steps)}


@pytest.mark.parametrize(
    ("job", "expected"),
    [
        (_job(INSTALL_STEP, CARGO_TEST), False),
        (_job(CARGO_TEST), True),
        (_job(CARGO_TEST, INSTALL_STEP), True),
        (_job({**INSTALL_STEP, "if": "always()"}, CARGO_TEST), True),
        (_job({**INSTALL_STEP, "continue-on-error": True}, CARGO_TEST), True),
        (
            _job({**INSTALL_STEP, "with": {"install-mold": "false"}}, CARGO_TEST),
            True,
        ),
        (_job(MAKE_TEST), True),
        (_job(INSTALL_STEP), False),
        (_job(CARGO_TEST, runs_on="windows-2022"), False),
    ],
    ids=[
        "provisioned",
        "no-install",
        "install-after-suite",
        "conditional-install",
        "soft-failing-install",
        "mold-not-installed",
        "new-make-suite-job",
        "not-a-suite-job",
        "non-linux-exception",
    ],
)
def test_the_provisioning_reader_reports_each_way_the_install_can_be_lost(
    job: Job, *, expected: bool
) -> None:
    """Every mutation that loses or delays the install is detected."""
    problems = provisioning_problems(job, depends_on_preflight())
    assert bool(problems) is expected, problems


@pytest.mark.parametrize(
    ("run", "expected"),
    [
        ("make -j 4 test", True),
        ("make -C . test", True),
        ("make CARGO=cargo test", True),
        ("make check-fmt test", True),
        ("make check-fmt && make test", True),
        ("make lint; make test", True),
        ("# make test", False),
    ],
    ids=[
        "job-count-option",
        "directory-option",
        "variable-assignment",
        "two-goals",
        "two-invocations-and",
        "two-invocations-semicolon",
        "shell-comment",
    ],
)
def test_the_reader_recognises_each_make_goal_form(run: str, *, expected: bool) -> None:
    """Every goal spelling is read, while a commented-out `make` is ignored."""
    job = _job({"name": "Test", "run": run})
    problems = provisioning_problems(job, depends_on_preflight())
    assert bool(problems) is expected, (run, problems)


@pytest.mark.parametrize(
    ("line", "expected"),
    [
        ("make -j 4 test", ["test"]),
        ("make -C . test", ["test"]),
        ("make --directory . test", ["test"]),
        ("make -f Makefile test", ["test"]),
        ("make CARGO=cargo test", ["test"]),
        ("make check-fmt test", ["check-fmt", "test"]),
        ("make check-fmt && make test", ["check-fmt", "test"]),
        ("make lint; make test", ["lint", "test"]),
        ("cargo test", []),
        ("# make test", []),
    ],
    ids=[
        "job-count-option",
        "directory-option",
        "separated-directory",
        "separated-makefile",
        "variable-assignment",
        "two-goals",
        "two-invocations-and",
        "two-invocations-semicolon",
        "not-make",
        "shell-comment",
    ],
)
def test_make_goals_reads_each_invocation_form(line: str, expected: list[str]) -> None:
    """The tokenizer returns exactly the goals of every `make` on the line."""
    assert make_goals(line) == expected, line


def test_prerequisite_map_accumulates_repeated_rules() -> None:
    """Two rules for one target contribute both prerequisite lists in order."""
    rules = [
        {"targets": ["lint"], "prerequisites": ["check-fmt"]},
        {"targets": ["lint"], "prerequisites": ["check-build-tools"]},
    ]
    parsed = prerequisite_map(rules)
    assert parsed["lint"] == ("check-fmt", "check-build-tools"), parsed
    assert depends_on_preflight(parsed)("lint") is True, (
        "the accumulated prerequisite must reach the preflight"
    )


def test_the_workflow_reader_rejects_duplicate_mapping_keys(tmp_path: Path) -> None:
    """A repeated key fails loudly, naming the key and its one-based line."""
    document = tmp_path / "duplicate.yml"
    document.write_text(
        "jobs:\n  example:\n    runs-on: ubuntu-latest\n  example: {}\n",
        "utf-8",
    )
    with pytest.raises(
        yaml.constructor.ConstructorError,
        match="duplicate key 'example' on line 4",
    ):
        load_workflow(document)


@pytest.mark.parametrize(
    ("runs_on", "matrix", "expected"),
    [
        ("ubuntu-latest", None, True),
        (["self-hosted", "linux"], None, True),
        ({"group": "linux-runners"}, None, True),
        ({"labels": ["ubuntu-latest"]}, None, True),
        ("windows-2022", None, False),
        ("macos-15-intel", None, False),
        (
            "${{ matrix.os }}",
            {"include": [{"os": "macos-15-intel"}, {"os": "windows-2022"}]},
            False,
        ),
        (
            "${{ matrix.os }}",
            {"include": [{"os": "macos-15-intel"}, {"os": "ubuntu-latest"}]},
            True,
        ),
    ],
    ids=[
        "scalar-linux",
        "list-linux",
        "mapping-group",
        "mapping-labels",
        "scalar-windows",
        "scalar-macos",
        "matrix-non-linux",
        "matrix-mixed",
    ],
)
def test_runner_forms_are_classified_by_platform(
    runs_on: object, matrix: dict[str, object] | None, *, expected: bool
) -> None:
    """Scalar, list and mapping runner forms are classified, matrix included."""
    job: Job = {"runs-on": runs_on, "steps": [INSTALL_STEP, CARGO_TEST]}
    if matrix is not None:
        job["strategy"] = {"matrix": matrix}
    assert is_linux_job(job) is expected, runner_values(job)
