"""Helpers for contract tests that parse the Makefile and CI workflows.

Makeutil parses the Makefile into structured rules and variables, so contract
tests can assert the build interface without depending on whitespace or nearby
source text. These helpers deliberately re-run the parser on every call: a
module-level cache would let one test observe another test's stale view of the
Makefile.
"""

from __future__ import annotations

import json
import shlex
import subprocess  # ruff: ignore[suspicious-subprocess-import] contract tests invoke the pinned parser
import typing as typ
from pathlib import Path

import yaml

MAKEUTIL_COMMAND: typ.Final = ("makeutil", "parse", "Makefile")


def repo_root() -> Path:
    """Return the repository root directory.

    Returns
    -------
    Path
        Absolute path of the directory containing the Makefile.

    Examples
    --------
    >>> (repo_root() / "Makefile").is_file()
    True
    """
    return Path(__file__).resolve().parents[1]


def makefile_report() -> dict[str, object]:
    """Return Makeutil's complete, successfully parsed Makefile report.

    Returns
    -------
    dict[str, object]
        The parsed report, with ``variables`` and ``rules`` arrays.

    Raises
    ------
    AssertionError
        If Makeutil recovers from (or fails) the parse instead of
        completing it.

    Examples
    --------
    >>> report = makefile_report()
    >>> "variables" in report and "rules" in report
    True
    """
    completed = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed parser command
        MAKEUTIL_COMMAND,
        capture_output=True,
        check=True,
        cwd=repo_root(),
        text=True,
    )
    report = typ.cast("dict[str, object]", json.loads(completed.stdout))
    parse = mapping(report.get("parse"), subject="parse report")
    if parse.get("status") != "complete":
        msg = f"makeutil did not complete the Makefile parse: {parse!r}"
        raise AssertionError(msg)
    return report


def mapping(value: object, *, subject: str) -> dict[str, object]:
    """Return a JSON object, naming the unexpected `subject` on failure."""
    if not isinstance(value, dict):
        msg = f"expected {subject} to be a JSON object"
        raise TypeError(msg)
    return typ.cast("dict[str, object]", value)


def objects(value: object, *, subject: str) -> list[dict[str, object]]:
    """Return a JSON object array, naming the unexpected `subject` on failure."""
    if not isinstance(value, list):
        msg = f"expected {subject} to be a JSON array"
        raise TypeError(msg)
    return [mapping(item, subject=f"{subject} item") for item in value]


def text_sequence(value: object, *, subject: str) -> tuple[str, ...]:
    """Return a JSON string array, naming the unexpected `subject` on failure."""
    if not isinstance(value, list):
        msg = f"expected {subject} to be a JSON array"
        raise TypeError(msg)
    if not all(isinstance(item, str) for item in value):
        msg = f"expected {subject} to contain only JSON strings"
        raise TypeError(msg)
    return tuple(typ.cast("list[str]", value))


def sole_variable(name: str) -> dict[str, object]:
    """Return Makeutil's sole variable fact for `name`."""
    variables = objects(makefile_report().get("variables"), subject="variables")
    matches = [variable for variable in variables if variable.get("name") == name]
    if len(matches) != 1:
        msg = f"expected one Makefile variable named {name!r}, found {len(matches)}"
        raise AssertionError(msg)
    return matches[0]


def variable_tokens(name: str) -> tuple[str, ...]:
    """Return shell-like tokens from Makeutil's raw variable value.

    Make line continuations survive `shlex.split` as bare newline tokens; they
    are layout, not arguments, so they are dropped here.

    Returns
    -------
    tuple[str, ...]
        The variable's shell-like tokens, free of Make line continuations.

    Raises
    ------
    TypeError
        If Makeutil did not report a string value for `name`.
    """
    value = sole_variable(name).get("raw_value")
    if not isinstance(value, str):
        msg = f"expected {name!r} to have a string value"
        raise TypeError(msg)
    return tuple(token for token in shlex.split(value) if token.strip())


def sole_recipe_rule(target: str, *, require_recipes: bool = True) -> dict[str, object]:
    """Return the only parsed rule for `target`, optionally requiring recipes."""
    rules = objects(makefile_report().get("rules"), subject="rules")
    matches = [
        rule
        for rule in rules
        if target in text_sequence(rule.get("targets"), subject="rule targets")
        and (
            not require_recipes or objects(rule.get("recipes"), subject="rule recipes")
        )
    ]
    if len(matches) != 1:
        msg = f"expected one Makefile rule named {target!r}, found {len(matches)}"
        raise AssertionError(msg)
    return matches[0]


def recipe_tokens(target: str) -> tuple[tuple[str, ...], ...]:
    """Return shell-like tokens for every recipe in `target`."""
    recipes = objects(
        sole_recipe_rule(target).get("recipes"), subject=f"{target} recipes"
    )
    return tuple(
        tuple(shlex.split(recipe_text))
        for recipe in recipes
        if isinstance(recipe_text := recipe.get("text"), str)
    )


def workflow_job(workflow_path: str, job_name: str) -> dict[str, object]:
    """Return the named job from a repository workflow."""
    workflow = yaml.safe_load((repo_root() / workflow_path).read_text(encoding="utf-8"))
    workflow_mapping = mapping(workflow, subject=f"{workflow_path} workflow")
    jobs = mapping(workflow_mapping.get("jobs"), subject=f"{workflow_path} jobs")
    return mapping(jobs.get(job_name), subject=f"{workflow_path} job {job_name!r}")


def sole_workflow_step(
    workflow_path: str, job_name: str, step_name: str
) -> dict[str, object]:
    """Return the sole named step from `job_name` in `workflow_path`."""
    job = workflow_job(workflow_path, job_name)
    steps = objects(job.get("steps"), subject=f"{workflow_path} job {job_name!r} steps")
    matches = [step for step in steps if step.get("name") == step_name]
    if len(matches) != 1:
        msg = (
            f"expected one {step_name!r} step in {workflow_path} job {job_name!r}, "
            f"found {len(matches)}"
        )
        raise AssertionError(msg)
    return matches[0]
