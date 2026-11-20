"""Readers that derive which hosted jobs run a preflight-gated suite.

The provisioning contract finds workflow jobs by reachability rather than by
name: a job matters when a suite step reaches Make's ``check-build-tools``
preflight, and it is correctly provisioned when an earlier step installs the
pinned toolchain and linker. These helpers answer both questions from the
Makefile and the workflow documents, without hard-coding today's job names.

A step whose shell text cannot be read is reported, not skipped: it might be
the suite step the install has to precede, and silence would read as success.
"""

from __future__ import annotations

import typing as typ

from tests.workflow_command_helpers import (
    UnreadableShellLineError,
    _goals,
    _line_tokens,
    _logical_lines,
    _runs_cargo_suite,
)
from tests.workflow_runner_helpers import is_linux_job

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from tests.workflow_document_helpers import Job

PREFLIGHT: typ.Final[str] = "check-build-tools"


def _reaches_preflight(
    target: str,
    prerequisites: cabc.Mapping[str, cabc.Sequence[str]],
    seen: frozenset[str],
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


def depends_on_preflight(
    prerequisites: cabc.Mapping[str, cabc.Sequence[str]],
) -> cabc.Callable[[str], bool]:
    """Return a predicate for Make targets that compile through the preflight.

    Parameters
    ----------
    prerequisites
        A parsed prerequisite map, such as ``prerequisite_map()`` or a
        deliberately synthetic one. Requiring it keeps Makeutil execution at
        the caller's explicit boundary instead of inside this query.

    Returns
    -------
    collections.abc.Callable[[str], bool]
        A predicate over Make target names.
    """
    parsed = prerequisites

    def depends(target: str) -> bool:
        """Report whether `target` compiles through the preflight."""
        return any(
            _reaches_preflight(nested, parsed, frozenset({target}))
            for nested in parsed.get(target, ())
        )

    return depends


class SuiteScan(typ.NamedTuple):
    """What a step's shell says about the suite, and what could not be read.

    ``reason`` is the line or action that runs the suite, if one does.
    ``unreadable`` holds every line the shell lexer rejected, because such a
    line may hide a suite command and cannot be treated as goal-free.
    """

    reason: str | None
    unreadable: tuple[str, ...]


def _names_suite(tokens: list[str], depends: cabc.Callable[[str], bool]) -> bool:
    """Report whether `tokens` run Cargo's suite or a preflight-gated goal."""
    return _runs_cargo_suite(tokens) or any(depends(goal) for goal in _goals(tokens))


def _scan_run(run: str, depends: cabc.Callable[[str], bool]) -> SuiteScan:
    """Read a step's ``run`` body for the suite, recording unreadable lines.

    Parameters
    ----------
    run
        A step's complete ``run`` body.
    depends
        The Make-preflight reachability predicate.

    Returns
    -------
    SuiteScan
        The first qualifying line, stripped, and the lines that would not
        tokenise. Both are needed: a failure is reported rather than read as
        an answer of "no suite here".
    """
    reason: str | None = None
    unreadable: list[str] = []
    for line in _logical_lines(run):
        try:
            tokens = _line_tokens(line)
        except UnreadableShellLineError:
            unreadable.append(line.strip())
            continue
        if reason is None and _names_suite(tokens, depends):
            reason = line.strip()
    return SuiteScan(reason=reason, unreadable=tuple(unreadable))


def _scan_step(step: Job, depends: cabc.Callable[[str], bool]) -> SuiteScan:
    """Return the suite reading of `step`, by its action or its shell text."""
    uses = str(step.get("uses", ""))
    if "generate-coverage" in uses:
        return SuiteScan(reason=uses, unreadable=())
    return _scan_run(str(step.get("run", "")), depends)


def _scanned_steps(
    job: Job, depends: cabc.Callable[[str], bool]
) -> list[tuple[int, Job, SuiteScan]]:
    """Return each step of `job` with its index and suite reading."""
    steps = [step for step in job.get("steps") or [] if isinstance(step, dict)]
    return [(at, step, _scan_step(step, depends)) for at, step in enumerate(steps)]


def _suite_readings(
    scanned: list[tuple[int, Job, SuiteScan]],
) -> list[tuple[int, str]]:
    """Return the index and command of every step in `scanned` that runs the suite."""
    return [(at, scan.reason) for at, _, scan in scanned if scan.reason is not None]


def _unreadable_complaints(scanned: list[tuple[int, Job, SuiteScan]]) -> list[str]:
    """Return one complaint per shell line in `scanned` that would not tokenise."""
    return [
        f"unparseable run line: {line}"
        for _, _, scan in scanned
        for line in scan.unreadable
    ]


def suite_steps(job: Job, depends: cabc.Callable[[str], bool]) -> list[tuple[int, str]]:
    """Return the steps of `job` that run the suite and why, in step order.

    Parameters
    ----------
    job
        The workflow job.
    depends
        The Make-preflight reachability predicate.

    Returns
    -------
    list[tuple[int, str]]
        The step index and the command or action that identifies it.
    """
    return _suite_readings(_scanned_steps(job, depends))


def _is_tool_install(step: Job) -> bool:
    """Report whether `step` installs the pinned toolchain and linker."""
    if "setup-rust" not in str(step.get("uses", "")):
        return False
    settings = step.get("with") or {}
    return str(settings.get("install-mold")).lower() == "true"


def _install_faults(steps: list[Job], at: int) -> list[str]:
    """Return complaints about how the install at `at` is declared.

    Parameters
    ----------
    steps
        The job's steps.
    at
        Index of an install step.

    Returns
    -------
    list[str]
        One complaint per lost guarantee.
    """
    faults = []
    if "if" in steps[at]:
        faults.append(f"install step {at} must be unconditional")
    if steps[at].get("continue-on-error"):
        faults.append(f"install step {at} must not be soft-failing")
    return faults


def _install_problems(
    steps: list[Job], installs: list[int], first_suite: int
) -> list[str]:
    """Return complaints about the position and declaration of the installs.

    Parameters
    ----------
    steps
        The job's steps.
    installs
        Indices of the installing steps.
    first_suite
        Index of the first suite step.

    Returns
    -------
    list[str]
        The complaints; empty when the installs precede the suite and are
        unconditional.
    """
    problems = []
    if not any(at < first_suite for at in installs):
        problems.append("the tool install must precede the first suite step")
    return problems + [fault for at in installs for fault in _install_faults(steps, at)]


def provisioning_problems(job: Job, depends: cabc.Callable[[str], bool]) -> list[str]:
    """Return one complaint per way a suite job fails to provision its tools.

    A ``run`` line the shell lexer rejects is a complaint of its own on a Linux
    job: it may be hiding the suite step the install has to precede, so it
    cannot be passed over as if it named nothing.

    Parameters
    ----------
    job
        The workflow job.
    depends
        The Make-preflight reachability predicate.

    Returns
    -------
    list[str]
        The complaints; empty when the job is not a Linux job or is correctly
        provisioned and fully readable.
    """
    if not is_linux_job(job):
        return []
    scanned = _scanned_steps(job, depends)
    unreadable = _unreadable_complaints(scanned)
    suite = _suite_readings(scanned)
    if not suite:
        return unreadable
    steps = [step for _, step, _ in scanned]
    installs = [at for at, step in enumerate(steps) if _is_tool_install(step)]
    if not installs:
        return [
            *unreadable,
            f"the suite runs with no setup-rust install-mold step: {suite}",
        ]
    return [
        *unreadable,
        *_install_problems(steps, installs, min(at for at, _ in suite)),
    ]
