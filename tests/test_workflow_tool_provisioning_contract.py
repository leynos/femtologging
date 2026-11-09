"""Contract tests for provisioning build tools before a hosted suite runs.

A coverage build may exclude ``mold`` from its compiler flags and still run
tests that reach Make's build-tool preflight, so every applicable Linux job
that runs the suite has to install the pinned toolchain and linker first.
These tests derive the affected jobs from workflow and Make reachability, then
prove the reader notices each way the installation can be lost.
"""

from __future__ import annotations

import re
import typing as typ
from pathlib import Path

import pytest
import yaml

from tests.make_contract_helpers import makefile_report, objects, text_sequence

if typ.TYPE_CHECKING:
    import collections.abc as cabc

ROOT: typ.Final[Path] = Path(__file__).resolve().parents[1]
WORKFLOW_DIR: typ.Final[Path] = ROOT / ".github" / "workflows"
PREFLIGHT: typ.Final[str] = "check-build-tools"
SETUP_RUST: typ.Final[str] = (
    "leynos/shared-actions/.github/actions/setup-rust@"
    "ff1dd759dfffc0db3459e30e833f52437ee62b57"
)
MAKE_TARGET: typ.Final[re.Pattern[str]] = re.compile(
    r"\bmake\s+(?:-\S+\s+)*([A-Za-z0-9][\w-]*)"
)
SUITE_CARGO: typ.Final[re.Pattern[str]] = re.compile(r"\bcargo\s+(?:test|nextest)\b")
type Job = dict[str, typ.Any]


def _prerequisite_map() -> dict[str, tuple[str, ...]]:
    """Return every Makefile target's direct prerequisites.

    Returns
    -------
    dict[str, tuple[str, ...]]
        The parsed prerequisite lists, keyed by target name.
    """
    rules = objects(makefile_report().get("rules"), subject="rules")
    return {
        target: text_sequence(
            rule.get("prerequisites"), subject=f"{target} prerequisites"
        )
        for rule in rules
        for target in text_sequence(rule.get("targets"), subject="rule targets")
    }


def _reaches_preflight(
    target: str, prerequisites: dict[str, tuple[str, ...]], seen: frozenset[str]
) -> bool:
    """Report whether `target` depends, directly or indirectly, on the preflight.

    Parameters
    ----------
    target
        A prerequisite to inspect.
    prerequisites
        The Makefile's prerequisite map.
    seen
        Targets already visited, which stops a cyclic declaration looping.

    Returns
    -------
    bool
        Whether `target` is, or depends on, ``check-build-tools``.
    """
    if target == PREFLIGHT:
        return True
    if target in seen:
        return False
    return any(
        _reaches_preflight(nested, prerequisites, seen | {target})
        for nested in prerequisites.get(target, ())
    )


def _depends_on_preflight() -> cabc.Callable[[str], bool]:
    """Return a predicate for Make targets that compile through the preflight.

    Returns
    -------
    collections.abc.Callable[[str], bool]
        A predicate over Make target names.
    """
    prerequisites = _prerequisite_map()

    def depends(target: str) -> bool:
        """Report whether `target` compiles through the preflight."""
        return any(
            _reaches_preflight(nested, prerequisites, frozenset({target}))
            for nested in prerequisites.get(target, ())
        )

    return depends


def _flatten(value: object) -> list[str]:
    """Return every scalar in a YAML value as text.

    Parameters
    ----------
    value
        A scalar, sequence or mapping from a workflow document.

    Returns
    -------
    list[str]
        The scalar values in document order.
    """
    match value:
        case str():
            return [value]
        case list() | tuple():
            return [text for item in value for text in _flatten(item)]
        case dict():
            return [text for item in value.values() for text in _flatten(item)]
        case _:
            return []


def _runner_values(job: Job) -> list[str]:
    """Return the runner labels and matrix operating systems of `job`."""
    values = _flatten(job.get("runs-on"))
    matrix = job.get("strategy", {}).get("matrix", {})
    if isinstance(matrix, dict):
        for key, value in matrix.items():
            if key == "include" and isinstance(value, list):
                entries = [entry for entry in value if isinstance(entry, dict)]
                values.extend(str(entry["os"]) for entry in entries if "os" in entry)
            elif key == "os":
                values.extend(_flatten(value))
    return values


def _is_linux_job(job: Job) -> bool:
    """Report whether `job` runs on Linux runners.

    A runner expression that no concrete label resolves (such as
    ``${{ matrix.os }}``) is left to the matrix values; an expression with no
    matrix detail is treated as Linux, because the repository's hosted suites
    run there. Non-Linux jobs are a legitimate exception: their platform
    linker replaces ``mold``.

    Returns
    -------
    bool
        Whether the job's concrete runners are Linux.
    """
    concrete = [value for value in _runner_values(job) if "${{" not in value]
    if not concrete:
        return True
    joined = " ".join(concrete).lower()
    if any(word in joined for word in ("ubuntu", "linux", "ubicloud")):
        return True
    return not any(word in joined for word in ("windows", "macos"))


def _suite_reasons(
    steps: list[Job], depends: cabc.Callable[[str], bool]
) -> list[tuple[int, str]]:
    """Return the steps that run the suite and why, in step order.

    Parameters
    ----------
    steps
        The job's steps.
    depends
        The Make-preflight reachability predicate.

    Returns
    -------
    list[tuple[int, str]]
        The step index and the command or action that identifies it.
    """
    reasons: list[tuple[int, str]] = []
    for at, step in enumerate(steps):
        uses = str(step.get("uses", ""))
        if "generate-coverage" in uses:
            reasons.append((at, uses))
            continue
        for line in str(step.get("run", "")).splitlines():
            if SUITE_CARGO.search(line):
                reasons.append((at, line.strip()))
                break
            match = MAKE_TARGET.search(line)
            if match is not None and depends(match.group(1)):
                reasons.append((at, line.strip()))
                break
    return reasons


def _is_tool_install(step: Job) -> bool:
    """Report whether `step` installs the pinned toolchain and linker."""
    if "setup-rust" not in str(step.get("uses", "")):
        return False
    settings = step.get("with") or {}
    return str(settings.get("install-mold")).lower() == "true"


def _provisioning_problems(job: Job, depends: cabc.Callable[[str], bool]) -> list[str]:
    """Return one complaint per way a suite job fails to provision its tools.

    Parameters
    ----------
    job
        The workflow job.
    depends
        The Make-preflight reachability predicate.

    Returns
    -------
    list[str]
        The complaints; empty when the job is not a Linux suite job or is
        correctly provisioned.
    """
    steps = [step for step in job.get("steps") or [] if isinstance(step, dict)]
    suite = _suite_reasons(steps, depends)
    if not suite or not _is_linux_job(job):
        return []
    installs = [at for at, step in enumerate(steps) if _is_tool_install(step)]
    if not installs:
        return [f"the suite runs with no setup-rust install-mold step: {suite}"]
    problems = []
    first_suite = min(at for at, _ in suite)
    if not any(at < first_suite for at in installs):
        problems.append("the tool install must precede the first suite step")
    for at in installs:
        if "if" in steps[at]:
            problems.append(f"install step {at} must be unconditional")
        if steps[at].get("continue-on-error"):
            problems.append(f"install step {at} must not be soft-failing")
    return problems


def _workflow_jobs() -> list[tuple[str, str, Job]]:
    """Return every concrete job of every repository workflow.

    A reusable-workflow call has no steps of its own, so its target file is
    resolved and must exist; otherwise the reachability reader would silently
    skip the jobs behind the call.

    Returns
    -------
    list[tuple[str, str, Job]]
        The workflow file name, job id and job document, in file order.
    """
    documents = sorted(WORKFLOW_DIR.glob("*.yml"))
    assert documents, "the repository must publish workflow files"
    jobs: list[tuple[str, str, Job]] = []
    for path in documents:
        document = yaml.safe_load(path.read_text("utf-8"))
        job_map = document.get("jobs") if isinstance(document, dict) else None
        assert isinstance(job_map, dict), f"{path.name} must declare a jobs mapping"
        for job_id, job in job_map.items():
            assert isinstance(job, dict), (
                f"{path.name} job {job_id!r} must be a mapping"
            )
            reusable = job.get("uses")
            if isinstance(reusable, str):
                if reusable.startswith("./"):
                    assert (ROOT / reusable).is_file(), (
                        f"{path.name} job {job_id!r} calls a missing workflow: "
                        f"{reusable}"
                    )
                # A remote reusable workflow's jobs are not in this repository;
                # its own contract owns the tools it installs.
                continue
            steps = job.get("steps")
            assert isinstance(steps, list), (
                f"{path.name} job {job_id!r} must declare a step list"
            )
            jobs.append((path.name, str(job_id), job))
    return jobs


def test_every_hosted_suite_job_provisions_the_build_tools() -> None:
    """Each Linux job running the suite installs the tools before it starts."""
    depends = _depends_on_preflight()
    suite_jobs = [
        (workflow, job_id, job)
        for workflow, job_id, job in _workflow_jobs()
        if _suite_reasons(
            [step for step in job.get("steps") or [] if isinstance(step, dict)],
            depends,
        )
    ]
    found = {(workflow, job_id) for workflow, job_id, _ in suite_jobs}
    assert {("ci.yml", "build-test"), ("heavy-tests.yml", "heavy")} <= found, (
        f"the reader missed a known suite job; it found {sorted(found)}"
    )
    problems = [
        f"{workflow} {job_id}: {problem}"
        for workflow, job_id, job in suite_jobs
        for problem in _provisioning_problems(job, depends)
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
    problems = _provisioning_problems(job, _depends_on_preflight())
    assert bool(problems) is expected, problems


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
    assert _is_linux_job(job) is expected, _runner_values(job)
