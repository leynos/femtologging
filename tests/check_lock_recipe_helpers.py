"""Read and judge the ``check-lock`` Makefile recipe.

The behavioural tests execute the real recipe, but they also need to prove
that the recipe's own shell branching is sound and that each mutation used to
probe it actually changes the text. That judgement is independent of any
temporary repository, so it lives here and is driven by the test module with
plain command lists.
"""

from __future__ import annotations

import collections.abc as cabc
import re
import typing as typ

from tests.make_contract_helpers import objects, sole_recipe_rule

CHECK_LOCK_TARGET: typ.Final = "check-lock"
LOCKFILE: typ.Final = "uv.lock"
IGNORED_MESSAGE: typ.Final = "uv.lock is ignored"
MASKED_PROBE: typ.Final[re.Pattern[str]] = re.compile(r"check-ignore[^\n]*\|\|\s*true")


def recipe_commands(target: str) -> list[str]:
    """Return the Makefile recipe for `target` as shell commands.

    The recipe is taken as makeutil reports it, so quoting and line
    continuations survive: tokenising with ``shlex`` would drop the quotes
    around the diagnostic messages. Make runs one shell per logical recipe
    line and stops at the first failure, so each command is returned
    separately, with make's ``$$`` escaping resolved to a literal ``$`` and
    its leading ``@`` removed.

    Parameters
    ----------
    target
        The Makefile target whose recipe to read.

    Returns
    -------
    list[str]
        One shell command per recipe line, in order.
    """
    recipes = objects(
        sole_recipe_rule(target).get("recipes"), subject=f"{target} recipes"
    )
    commands = []
    for recipe in recipes:
        text = recipe.get("text")
        if not isinstance(text, str):
            continue
        line = text.replace("$$", "$").removeprefix("@")
        if line.strip() and not line.lstrip().startswith("#"):
            commands.append(line)
    return commands


def recipe_problems(commands: list[str]) -> list[str]:
    """Return one complaint per way the gate's status branching is unsound.

    The branching is judged rather than described: a mutant recipe must draw a
    complaint, so a check that reports nothing cannot pass by accident.

    Parameters
    ----------
    commands
        The recipe's shell commands, in order.

    Returns
    -------
    list[str]
        One complaint per requirement the recipe fails to impose.
    """
    script = "\n".join(commands)
    problems: list[str] = []
    if not re.search(r"git check-ignore --no-index -q uv\.lock", script):
        problems.append("the ignore probe must run quietly against uv.lock")
    if "status -eq 0" not in script:
        problems.append("a matching ignore rule must fail the gate")
    if "status -ne 1" not in script:
        problems.append("a probe error other than 'not ignored' must propagate")
    if MASKED_PROBE.search(script):
        problems.append("the probe's status must not be masked")
    if "git ls-files --error-unmatch uv.lock" not in script:
        problems.append("the gate must require Git to track the lock")
    return problems


def unquiet_probe(commands: list[str]) -> list[str]:
    """Drop the probe's quiet flag."""
    return [
        command.replace("check-ignore --no-index -q", "check-ignore --no-index")
        for command in commands
    ]


def inverted_match_branch(commands: list[str]) -> list[str]:
    """Treat the 'ignored' status as success."""
    return [command.replace("status -eq 0", "status -eq 1") for command in commands]


def dropped_error_branch(commands: list[str]) -> list[str]:
    """Remove the branch that propagates an unexpected status."""
    return [
        re.sub(r"if \[ \$status -ne 1 \]; then.*?\n\s*fi", "", command, flags=re.DOTALL)
        for command in commands
    ]


def masked_probe(commands: list[str]) -> list[str]:
    """Swallow the probe's exit status."""
    return [
        command.replace(
            "git check-ignore --no-index -q uv.lock; status=$?",
            "git check-ignore --no-index -q uv.lock || true; status=$?",
        )
        for command in commands
    ]


def untracked_check_removed(commands: list[str]) -> list[str]:
    """Drop the requirement that Git tracks the lock."""
    return [
        command for command in commands if "git ls-files --error-unmatch" not in command
    ]


type RecipeMutation = cabc.Callable[[list[str]], list[str]]

MUTANTS: typ.Final[tuple[tuple[RecipeMutation, str], ...]] = (
    (unquiet_probe, "unquiet-probe"),
    (inverted_match_branch, "inverted-match-branch"),
    (dropped_error_branch, "dropped-error-branch"),
    (masked_probe, "masked-probe"),
    (untracked_check_removed, "untracked-check-removed"),
)
