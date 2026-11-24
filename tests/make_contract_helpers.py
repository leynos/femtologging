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
import yaml.constructor
import yaml.resolver

if typ.TYPE_CHECKING:
    import collections.abc as cabc

MAKEUTIL_COMMAND: typ.Final = ("makeutil", "parse")


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


def makefile_report(path: str | Path = "Makefile") -> dict[str, object]:
    """Return Makeutil's complete, successfully parsed report for `path`.

    Parameters
    ----------
    path
        Makefile to parse, relative to the repository root unless absolute.

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
        (*MAKEUTIL_COMMAND, str(path)),
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


def prerequisite_map(
    rules: cabc.Sequence[cabc.Mapping[str, object]] | None = None,
) -> dict[str, tuple[str, ...]]:
    """Return every Makefile target's prerequisites, accumulated in order.

    Make accepts a target being declared by several rules, and their
    prerequisite lists combine. Reading each rule independently and letting a
    later one replace an earlier one would hide a dependency such as the
    build-tool preflight, so the lists are concatenated in declaration order.

    Parameters
    ----------
    rules
        Parsed Makeutil rule objects. When omitted, the repository's Makefile
        is parsed on this call, so no stale module-level cache is observed.

    Returns
    -------
    dict[str, tuple[str, ...]]
        The combined prerequisite lists, keyed by target name.
    """
    if rules is None:
        rules = objects(makefile_report().get("rules"), subject="rules")
    prerequisites: dict[str, list[str]] = {}
    for rule in rules:
        targets = text_sequence(rule.get("targets"), subject="rule targets")
        required = text_sequence(
            rule.get("prerequisites"), subject="rule prerequisites"
        )
        for target in targets:
            prerequisites.setdefault(target, []).extend(required)
    return {target: tuple(names) for target, names in prerequisites.items()}


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


def environment_overridable_variables() -> tuple[str, ...]:
    """Return the Makefile variables an inherited environment value replaces.

    A `?=` assignment applies only when the variable is unset, and the
    environment counts as set, so an exported value wins over the Makefile's
    default. Every other assignment operator defines the value outright. A
    contract test that runs a target through `make` therefore has to clear
    exactly these names to see the defaults it reads from the Makefile.

    Returns
    -------
    tuple[str, ...]
        The names of variables assigned with `?=`, in Makefile order.
    """
    variables = objects(makefile_report().get("variables"), subject="variables")
    return tuple(
        typ.cast("str", variable["name"])
        for variable in variables
        if variable.get("operator") == "?="
    )


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


def command_words(tokens: tuple[str, ...]) -> tuple[str, ...]:
    """Return the tokens after any leading ``NAME=value`` assignments.

    A shell command may be prefixed with environment assignments, so a rule
    that asks what program a step runs has to skip them first.

    Returns
    -------
    tuple[str, ...]
        The command and its arguments, or ``()`` when the command is nothing
        but assignments.
    """
    for index, token in enumerate(tokens):
        if "=" not in token.split(" ", 1)[0]:
            return tokens[index:]
    return ()


def defaults_shell(container: cabc.Mapping[str, object]) -> object:
    """Return a workflow or job ``defaults.run.shell`` value, if one is set.

    Returns
    -------
    object
        The configured shell template, or ``None`` when no default is set.
    """
    defaults = container.get("defaults")
    if not isinstance(defaults, dict):
        return None
    run = defaults.get("run")
    if not isinstance(run, dict):
        return None
    return run.get("shell")


def strict_safe_load(source: str) -> object:
    """Load YAML with `safe_load`, refusing any repeated mapping key.

    `safe_load` cannot be told to use a subclass, so the strict constructor is
    registered on a private subclass and restored afterwards; the global
    `SafeLoader` is never modified.

    Returns
    -------
    object
        The parsed document; the strict constructor raises
        ``ConstructorError`` when a key is repeated.
    """
    original = yaml.SafeLoader.yaml_constructors[
        yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG
    ]
    yaml.SafeLoader.add_constructor(
        yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, construct_mapping
    )
    try:
        return yaml.safe_load(source)
    finally:
        yaml.SafeLoader.add_constructor(
            yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, original
        )


def construct_mapping(
    loader: yaml.SafeLoader,
    node: yaml.nodes.MappingNode,
    *,
    deep: bool = False,
) -> dict[object, object]:
    """Build a mapping, refusing any key that appears twice.

    Returns
    -------
    dict[object, object]
        The constructed mapping.

    Raises
    ------
    DuplicateKeyError
        If two keys in the node are equal.
    """
    loader.flatten_mapping(node)
    seen: set[object] = set()
    for key_node, _ in node.value:
        key = loader.construct_object(key_node, deep=deep)
        if key in seen:
            raise DuplicateKeyError(key, node, key_node)
        seen.add(key)
    return typ.cast(
        "dict[object, object]", yaml.SafeLoader.construct_mapping(loader, node, deep)
    )


class DuplicateKeyError(yaml.constructor.ConstructorError):
    """Report a repeated mapping key and the position of its second use."""

    def __init__(
        self,
        key: object,
        node: yaml.nodes.MappingNode,
        key_node: yaml.nodes.Node,
    ) -> None:
        """Record the key and the position of its second use.

        Parameters
        ----------
        key
            The repeated key.
        node
            The mapping node that contains the repeat.
        key_node
            The node of the second occurrence.
        """
        super().__init__(
            "while constructing a mapping",
            node.start_mark,
            f"found duplicate key {key!r}",
            key_node.start_mark,
        )
