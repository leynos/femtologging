"""The runner-placement judgement: which runner a `runs-on` expression selects.

A lane that names an Ubicloud runner selects it by an expression that falls
back to the hosted pool for a fork or a Dependabot pull request. This module
holds that judgement; `test_runner_placement_contract.py` drives it over
constructed expressions and then over the real workflows.
"""

from __future__ import annotations

import re
import typing as typ

HOSTED_LABEL: typ.Final = "ubuntu-latest"
FORK_CONDITION: typ.Final = "github.event.pull_request.head.repo.fork"
#: The pull request's author, not `github.actor`: the actor is whoever
#: triggered the run, so a person pushing to a Dependabot branch would flip
#: the runner.
DEPENDABOT_CONDITION: typ.Final = (
    "github.event.pull_request.user.login == 'dependabot[bot]'"
)
HOSTED_CONDITION: typ.Final = f"({FORK_CONDITION} || {DEPENDABOT_CONDITION})"

#: Every job that can land on Ubicloud: workflow, job, runner class and the
#: ceiling it states in minutes. The inventory is exact, so a new Ubicloud lane
#: without a ceiling, or a class or ceiling changed, fails until reviewed.
PLACEMENTS: typ.Final = (
    ("build-wheels.yml", "build-linux", "ubicloud-standard-4", 40),
    ("build-wheels.yml", "verify-wheel-install", "ubicloud-standard-4", 40),
    ("ci.yml", "build-test", "ubicloud-standard-4", 40),
)

#: The runner-selection shape: a condition, a quoted hosted arm and a quoted
#: other arm.
_SHAPE: typ.Final = re.compile(
    r"\$\{\{\s*(?P<condition>\(.+?\))\s*&&\s*'(?P<hosted>[^']*)'"
    r"\s*\|\|\s*'(?P<other>[^']*)'\s*\}\}"
)

type Origin = typ.Literal["push", "same-repository", "fork", "dependabot"]
ORIGINS: typ.Final[tuple[Origin, ...]] = (
    "push",
    "same-repository",
    "fork",
    "dependabot",
)
HOSTED_ORIGINS: typ.Final[frozenset[Origin]] = frozenset({"fork", "dependabot"})


def estate_expression(label: str) -> str:
    """Return the runner-selection expression for an Ubicloud `label`.

    Parameters
    ----------
    label
        The Ubicloud runner class a non-fork run selects.

    Returns
    -------
    str
        A `${{ ... }}` expression selecting `label` unless the run is a pull
        request from a fork, which falls back to the hosted pool.
    """
    return f"${{{{ {HOSTED_CONDITION} && '{HOSTED_LABEL}' || '{label}' }}}}"


def selected_runner(runs_on: object, origin: Origin) -> str | None:
    """Return the label a `runs-on` value selects for a run.

    Parameters
    ----------
    runs_on
        The job's `runs-on` value as parsed.
    origin
        The kind of run: a push or dispatch (no pull request), a pull request
        from this repository, or a pull request from a fork.

    Returns
    -------
    str | None
        The selected label, or None when the value is not the runner-selection
        shape. A literal label is not the shape: a lane that never falls back
        cannot serve a fork.
    """
    shape = _SHAPE.fullmatch(runs_on.strip()) if isinstance(runs_on, str) else None
    if shape is None or shape["condition"] != HOSTED_CONDITION:
        return None
    return shape["hosted" if origin in HOSTED_ORIGINS else "other"]


def placement_faults(runs_on: object, label: str) -> list[str]:
    """Return one entry per kind of run the expression places wrongly.

    Parameters
    ----------
    runs_on
        The job's `runs-on` value as parsed.
    label
        The Ubicloud runner class every non-fork run must select.

    Returns
    -------
    list[str]
        Empty when a fork falls back to hosted and every other run is on
        `label`.
    """
    wanted = {
        "push": label,
        "same-repository": label,
        "fork": HOSTED_LABEL,
        "dependabot": HOSTED_LABEL,
    }
    return [
        f"{origin} selects {selected_runner(runs_on, origin)}, wanted {wanted[origin]}"
        for origin in ORIGINS
        if selected_runner(runs_on, origin) != wanted[origin]
    ]
