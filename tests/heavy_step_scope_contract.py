"""Rules for how the Loom execution step is actually run.

A workflow can keep a step's command text intact and still prevent it from
running: an overridden `shell` template parses the script without executing
it, and a `defaults.run.shell` does the same for every step of a job or a
workflow. These rules refuse an override at each scope.

The document accessors live in :mod:`tests.heavy_workflow_document`.
"""

from __future__ import annotations

import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

from tests.heavy_workflow_document import (
    HEAVY_JOB,
    LOOM_STEP,
    WORKFLOW,
    Job,
    steps,
)


def _defaults_shell(container: cabc.Mapping[str, object]) -> object:
    """Return a workflow or job ``defaults.run.shell`` value, if one is set."""
    defaults = container.get("defaults")
    if not isinstance(defaults, dict):
        return None
    run = defaults.get("run")
    if not isinstance(run, dict):
        return None
    return run.get("shell")


def _execution_step(found: Job) -> dict[str, object] | None:
    """Return the Loom execution step, or ``None`` when the job has none."""
    return next(
        (
            step
            for step in steps(found, subject=HEAVY_JOB)
            if step.get("name") == LOOM_STEP
        ),
        None,
    )


def shell_problems(document: cabc.Mapping[str, object], found: Job) -> list[str]:
    """Return complaints about overrides of how the execution step runs.

    A ``shell`` template such as ``bash -n {0}`` makes GitHub parse the script
    without executing it, so the step succeeds while no model runs. The
    effective shell is the step's own setting, else the job's
    ``defaults.run.shell``, else the workflow's; all three are refused.

    Parameters
    ----------
    document : dict[str, object]
        The parsed workflow, for the workflow-level defaults.
    found : Job
        The parsed heavy job.

    Returns
    -------
    list[str]
        One complaint per scope that overrides the shell.
    """
    problems: list[str] = []
    step = _execution_step(found)
    if step is not None and "shell" in step:
        problems.append(f"the {LOOM_STEP!r} step must not override the shell")
    if _defaults_shell(found) is not None:
        problems.append(f"the {HEAVY_JOB!r} job must not set defaults.run.shell")
    if _defaults_shell(document) is not None:
        problems.append(f"{WORKFLOW} must not set defaults.run.shell")
    return problems
