"""Readers that derive which hosted jobs run a preflight-gated suite.

The provisioning contract finds workflow jobs by reachability rather than by
name: a job matters when a suite step reaches Make's ``check-build-tools``
preflight, and it is correctly provisioned when an earlier step installs the
pinned toolchain and linker. These helpers answer both questions from the
Makefile and the workflow documents, without hard-coding today's job names.
"""

from __future__ import annotations

import re
import shlex
import typing as typ

from tests.make_contract_helpers import prerequisite_map
from tests.workflow_document_helpers import is_linux_job

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from tests.workflow_document_helpers import Job

PREFLIGHT: typ.Final[str] = "check-build-tools"
SUITE_CARGO: typ.Final[re.Pattern[str]] = re.compile(r"\bcargo\s+(?:test|nextest)\b")
CONTROL_OPERATORS: typ.Final[frozenset[str]] = frozenset({
    "&&",
    "||",
    ";",
    "|",
    "&",
    "(",
    ")",
})
# Options whose following token is a value rather than a goal. Long forms are
# usually written with `=`, which the goal filter drops on its own.
VALUE_OPTIONS: typ.Final[frozenset[str]] = frozenset({
    "-C",
    "-f",
    "-I",
    "-o",
    "-W",
    "--directory",
    "--file",
    "--makefile",
    "--include-dir",
    "--old-file",
    "--assume-old",
    "--new-file",
    "--assume-new",
    "--eval",
})


def _line_tokens(line: str) -> list[str]:
    """Return shell-like tokens for `line`, with comments removed."""
    lexer = shlex.shlex(line, posix=True, punctuation_chars=True)
    lexer.whitespace_split = True
    lexer.commenters = "#"
    try:
        return list(lexer)
    except ValueError:
        return []


def _command_segments(tokens: list[str]) -> list[list[str]]:
    """Split `tokens` into commands at shell control operators."""
    segments: list[list[str]] = []
    current: list[str] = []
    for token in tokens:
        if token in CONTROL_OPERATORS:
            segments.append(current)
            current = []
        else:
            current.append(token)
    segments.append(current)
    return segments


def _is_goal(token: str) -> bool:
    """Report whether `token` names a Make goal rather than an option."""
    return not token.startswith("-") and "=" not in token and not token.isdigit()


def _arguments_after_make(tokens: list[str]) -> cabc.Iterator[str]:
    """Yield Make's argument tokens, dropping an option's separate value."""
    index = tokens.index("make") + 1
    while index < len(tokens):
        if tokens[index] in VALUE_OPTIONS:
            index += 2
            continue
        yield tokens[index]
        index += 1


def _segment_goals(tokens: list[str]) -> list[str]:
    """Return the goals of one command's tokens, or none without a `make`."""
    if "make" not in tokens:
        return []
    return [token for token in _arguments_after_make(tokens) if _is_goal(token)]


def make_goals(line: str) -> list[str]:
    """Return every Make goal named by the shell `line`, in order.

    A step may run several commands on one line, so the token stream is split
    at shell control operators and each command is read separately. Options,
    option values, variable assignments and bare job counts are not goals.

    Parameters
    ----------
    line
        One line of a workflow step's ``run`` body.

    Returns
    -------
    list[str]
        The goals of every `make` invocation, concatenated in order.

    Examples
    --------
    >>> make_goals("make -C . check-fmt && make test")
    ['check-fmt', 'test']
    """
    tokens = _line_tokens(line)
    return [
        goal
        for segment in _command_segments(tokens)
        for goal in _segment_goals(segment)
    ]


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
    prerequisites: cabc.Mapping[str, cabc.Sequence[str]] | None = None,
) -> cabc.Callable[[str], bool]:
    """Return a predicate for Make targets that compile through the preflight.

    Parameters
    ----------
    prerequisites
        A parsed prerequisite map to query instead of the repository's
        Makefile. Tests inject a synthetic map; production callers omit it and
        the Makefile is parsed on this call.

    Returns
    -------
    collections.abc.Callable[[str], bool]
        A predicate over Make target names.
    """
    parsed = prerequisite_map() if prerequisites is None else prerequisites

    def depends(target: str) -> bool:
        """Report whether `target` compiles through the preflight."""
        return any(
            _reaches_preflight(nested, parsed, frozenset({target}))
            for nested in parsed.get(target, ())
        )

    return depends


def _run_suite_reason(run: str, depends: cabc.Callable[[str], bool]) -> str | None:
    """Return the line of `run` that starts a preflight-gated suite, if any.

    Parameters
    ----------
    run
        A step's complete ``run`` body.
    depends
        The Make-preflight reachability predicate.

    Returns
    -------
    str | None
        The first qualifying line, stripped, or ``None``.
    """
    for line in run.splitlines():
        if SUITE_CARGO.search(line):
            return line.strip()
        if any(depends(goal) for goal in make_goals(line)):
            return line.strip()
    return None


def _suite_reason(step: Job, depends: cabc.Callable[[str], bool]) -> str | None:
    """Return why `step` runs the suite, or ``None`` when it does not."""
    uses = str(step.get("uses", ""))
    if "generate-coverage" in uses:
        return uses
    return _run_suite_reason(str(step.get("run", "")), depends)


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
    steps = [step for step in job.get("steps") or [] if isinstance(step, dict)]
    reasons = (
        (at, reason)
        for at, step in enumerate(steps)
        if (reason := _suite_reason(step, depends)) is not None
    )
    return list(reasons)


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
    suite = suite_steps(job, depends)
    if not suite or not is_linux_job(job):
        return []
    installs = [at for at, step in enumerate(steps) if _is_tool_install(step)]
    if not installs:
        return [f"the suite runs with no setup-rust install-mold step: {suite}"]
    return _install_problems(steps, installs, min(at for at, _ in suite))
