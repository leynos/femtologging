"""Rules for how the heavy job's Loom steps are actually run.

A workflow can keep a step's command text intact and still prevent it from
running: an overridden `shell` template parses the script without executing
it, and a `defaults.run.shell` does the same for every step of a job or a
workflow. The compile step can likewise echo its command instead of building.
These rules refuse those shapes and provide the shell-aware tokenizer the
execution-step rules share.

The document accessors live in :mod:`tests.heavy_workflow_document`.
"""

from __future__ import annotations

import re
import shlex
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

from tests.heavy_workflow_document import (
    COMPILE_STEP,
    HEAVY_JOB,
    LOOM_STEP,
    WORKFLOW,
    Job,
    run_text,
    steps,
)
from tests.make_contract_helpers import command_words, defaults_shell

#: The steps the scheduled job must keep and must not make skippable.
LOOM_GATE_STEPS: tuple[str, ...] = (COMPILE_STEP, LOOM_STEP)

#: Operators that also bind without surrounding whitespace. The shell keeps
#: `&wait` attached to the preceding word, so each token is split on these
#: before it is judged.
_ATTACHED_OPERATORS: typ.Final = re.compile(r"([&;|])")

#: The `cargo test` subcommand arguments the compile step must carry.
_HEAVY_TARGET: tuple[str, ...] = ("--test", "heavy")
_NO_RUN: str = "--no-run"

#: The configuration the compile step must build under.
_LOOM_CONFIG: str = "--cfg loom"
_LOOM_CONFIG_FORM: str = "--cfg=loom"

#: The flag variable and its higher-precedence encoded form.
_FLAGS_VARIABLE: str = "RUSTFLAGS"
_ENCODED_FLAGS: str = "CARGO_ENCODED_RUSTFLAGS"

#: Shell control operators and the `wait` builtin: a compile step must be one
#: foreground command whose failure the job sees.
_CONTROL_TOKENS: tuple[str, ...] = ("&", ";", "|", "&&", "||", "wait")

#: Options that make Cargo print help instead of compiling.
_EARLY_EXITS: tuple[str, ...] = ("--help", "-h")


def run_tokens(step: dict[str, object]) -> tuple[str, ...]:
    """Return the shell command of a step as operator-separated tokens.

    A shell separates `&`, `;` and `|` from their neighbours, with or without
    whitespace, so the lexical tokens are split again on those characters. A
    command ending in `&wait`, or `true;handler`, then exposes the operator and
    the builtin the foreground rule looks for. An unquoted `#` ends the
    command, so a comment-only script tokenizes to nothing.

    Returns
    -------
    tuple[str, ...]
        The step's command tokens, with line continuations flattened and
        attached control operators separated.
    """
    command = run_text(step).strip()
    if "\n" in command.replace("\\\n", ""):
        # An unescaped newline is a command separator. shlex would discard it
        # and read an assignment line as part of the next command, so return a
        # token no rule accepts.
        return ("\n",)
    lexer = shlex.shlex(command.replace("\\\n", " "), posix=True)
    lexer.whitespace_split = True
    lexer.commenters = "#"
    return tuple(
        part for word in lexer for part in _ATTACHED_OPERATORS.split(word) if part
    )


def shell_problems(document: cabc.Mapping[str, object], found: Job) -> list[str]:
    """Return complaints about overrides of how the Loom steps run.

    A ``shell`` template such as ``bash -n {0}`` makes GitHub parse the script
    without executing it, so the step succeeds while nothing runs. The
    effective shell is the step's own setting, else the job's
    ``defaults.run.shell``, else the workflow's; all three are refused, for
    both named Loom steps.

    Returns
    -------
    list[str]
        One complaint per scope or step that overrides the shell.
    """
    problems: list[str] = []
    named = {step.get("name"): step for step in steps(found, subject=HEAVY_JOB)}
    for step_name in LOOM_GATE_STEPS:
        step = named.get(step_name)
        if step is not None and "shell" in step:
            problems.append(f"the {step_name!r} step must not override the shell")
    if defaults_shell(found) is not None:
        problems.append(f"the {HEAVY_JOB!r} job must not set defaults.run.shell")
    if defaults_shell(document) is not None:
        problems.append(f"{WORKFLOW} must not set defaults.run.shell")
    return problems


def loom_condition_problems(found: Job) -> list[str]:
    """Return one complaint per condition on the heavy job or a Loom step.

    A condition is never judged by its value: ``if: false`` and a
    prior-success expression skip the gate equally, and the workflow's step
    conditions are evaluated before the step runs. A Loom gate that can be
    skipped while the job reports success is not a gate.

    Returns
    -------
    list[str]
        A complaint for the job or each named Loom step that carries ``if``.
    """
    problems: list[str] = []
    if "if" in found:
        problems.append(f"the {HEAVY_JOB!r} job must carry no condition")
    found_steps = steps(found, subject=HEAVY_JOB)
    problems.extend(
        f"the Loom step {step.get('name')!r} must carry no condition"
        for step in found_steps
        if step.get("name") in LOOM_GATE_STEPS and "if" in step
    )
    return problems


def loom_tolerance_problems(found: Job) -> list[str]:
    """Return one complaint per Loom gate that tolerates its own failure.

    The heavy job may not tolerate failure at all, and neither named Loom step
    may. Without this rule, ``continue-on-error: true`` on the execution step
    would report a failing model run as a successful job.

    Returns
    -------
    list[str]
        A complaint for the job or each named Loom step that tolerates
        failure.
    """
    problems: list[str] = []
    if found.get("continue-on-error", False) is not False:
        problems.append(f"the {HEAVY_JOB!r} job must not tolerate failure")
    found_steps = steps(found, subject=HEAVY_JOB)
    problems.extend(
        f"the Loom step {step.get('name')!r} must not tolerate failure"
        for step in found_steps
        if step.get("name") in LOOM_GATE_STEPS
        and step.get("continue-on-error", False) is not False
    )
    return problems


def compile_command_problems(found: Job) -> list[str]:
    """Return a complaint unless the compile step runs the heavy test build.

    A step that keeps its name and its flags but echoes the command compiles
    nothing, and no other rule reads that step's program. The command must be
    a real ``cargo test`` for the heavy target with ``--no-run``.

    Returns
    -------
    list[str]
        A complaint when the compile step's program, target or ``--no-run``
        flag is missing.
    """
    step = next(
        (
            step
            for step in steps(found, subject=HEAVY_JOB)
            if step.get("name") == COMPILE_STEP
        ),
        None,
    )
    if step is None:
        return [f"the job must have a {COMPILE_STEP!r} step"]
    words = command_words(run_tokens(step))
    return _compile_words_problems(words)


def _compile_words_problems(words: tuple[str, ...]) -> list[str]:
    """Return the complaints about the compile step's command tokens.

    Returns
    -------
    list[str]
        A complaint for a masked or backgrounded command, an early-exit
        option, a program other than ``cargo test``, a missing heavy target or
        a missing ``--no-run``.
    """
    if any(token in _CONTROL_TOKENS for token in words):
        return [
            (
                f"the {COMPILE_STEP!r} step must run one foreground command "
                "whose status is not masked"
            )
        ]
    if any(option in _EARLY_EXITS for option in words):
        return [f"the {COMPILE_STEP!r} step must not ask Cargo for help"]
    if words[:2] != ("cargo", "test"):
        return [
            (
                f"the {COMPILE_STEP!r} step must run `cargo test`, its command "
                f"begins {words[:3]!r}"
            )
        ]
    # Cargo's own arguments end at `--`; anything after it belongs to the
    # test binary, so `--test heavy --no-run` there selects no target and
    # compiles nothing.
    cargo_arguments = words[: words.index("--")] if "--" in words else words
    if not _contains_in_order(cargo_arguments, _HEAVY_TARGET):
        return [f"the {COMPILE_STEP!r} step must name the heavy target"]
    if _NO_RUN not in cargo_arguments:
        return [f"the {COMPILE_STEP!r} step must compile without running"]
    return []


def _contains_in_order(tokens: tuple[str, ...], wanted: tuple[str, ...]) -> bool:
    """Return whether `wanted` appears in `tokens` as a contiguous run.

    Returns
    -------
    bool
        True when the run is present.
    """
    width = len(wanted)
    return any(
        tokens[start : start + width] == wanted
        for start in range(len(tokens) - width + 1)
    )


def compile_loom_flag_problems(
    document: cabc.Mapping[str, object], found: Job
) -> list[str]:
    """Return a complaint unless the compile step builds under `--cfg loom`.

    The step selects Loom through an inline ``RUSTFLAGS`` assignment, which
    the command-word helper deliberately skips. Without the flag the step
    compiles the ordinary heavy tests while still carrying the Loom gate's
    name, so the models never build. ``CARGO_ENCODED_RUSTFLAGS`` outranks
    ``RUSTFLAGS`` and is refused at every scope, as in the pull-request gate.

    Returns
    -------
    list[str]
        A complaint when no effective ``RUSTFLAGS`` carries ``--cfg loom``, or
        when an encoded value could displace it.
    """
    step = next(
        (
            step
            for step in steps(found, subject=HEAVY_JOB)
            if step.get("name") == COMPILE_STEP
        ),
        None,
    )
    if step is None:
        return [f"the job must have a {COMPILE_STEP!r} step"]
    problems: list[str] = []
    flags = _effective_value(step, found, document, _FLAGS_VARIABLE)
    if flags is None or not _enables_loom(flags):
        problems.append(
            f"the {COMPILE_STEP!r} step must set {_LOOM_CONFIG} in {_FLAGS_VARIABLE}"
        )
    encodings = (
        ("step", step),
        ("job", found),
        ("workflow", document),
    )
    problems.extend(
        f"the {COMPILE_STEP!r} step's {scope} must not set {_ENCODED_FLAGS}"
        for scope, container in encodings
        if _declares_environment(container, _ENCODED_FLAGS)
    )
    if _ENCODED_FLAGS in _inline_assignments(run_tokens(step)):
        problems.append(
            f"the {COMPILE_STEP!r} step must not set {_ENCODED_FLAGS} in its command"
        )
    return problems


def _enables_loom(flags: str) -> bool:
    """Return whether compiler flags enable exactly the `loom` configuration.

    The configuration name is a whole argument, so `--cfg loom_extra` must not
    satisfy it. Both `--cfg loom` and `--cfg=loom` are accepted.

    Returns
    -------
    bool
        True when the flags name the `loom` configuration.
    """
    words = shlex.split(flags)
    if _LOOM_CONFIG_FORM in words:
        return True
    return any(
        words[index : index + 2] == ["--cfg", "loom"] for index in range(len(words) - 1)
    )


def _effective_value(
    step: dict[str, object],
    found: Job,
    document: cabc.Mapping[str, object],
    name: str,
) -> str | None:
    """Return the value a shell step's environment variable takes.

    An inline ``NAME=value`` assignment in the command wins over step, job and
    workflow environments, in that order. The first scope that declares the
    variable decides, even when its value is not a string.

    Returns
    -------
    str | None
        The effective value, or ``None`` when no scope declares it or the
        declaring scope's value is not a string.
    """
    inline = _inline_assignments(run_tokens(step)).get(name)
    if inline is not None:
        return inline
    for container in (step, found, document):
        if not _declares_environment(container, name):
            continue
        # The first scope that declares the variable decides. A null or
        # non-string declaration still exports an empty value, so it must not
        # fall through to a broader scope that happens to carry a valid one.
        return _environment_value(container, name)
    return None


def _inline_assignments(tokens: tuple[str, ...]) -> dict[str, str]:
    """Return the ``NAME=value`` tokens before the command.

    Returns
    -------
    dict[str, str]
        The assignments, mapped by name.
    """
    assignments: dict[str, str] = {}
    for token in tokens:
        name, separator, value = token.partition("=")
        if not separator or " " in name:
            break
        assignments[name] = value
    return assignments


def _environment_value(container: cabc.Mapping[str, object], name: str) -> str | None:
    """Return a container's environment value for `name`, if it is a string.

    Returns
    -------
    str | None
        The configured string, or ``None`` when absent or not a string.
    """
    environment = container.get("env")
    if not isinstance(environment, dict):
        return None
    value = environment.get(name)
    return value if isinstance(value, str) else None


def _declares_environment(container: cabc.Mapping[str, object], name: str) -> bool:
    """Return whether a container's environment declares `name`.

    The key's presence is what matters: a null declaration still exports an
    empty value, which would displace the inline flags just as a value does.

    Returns
    -------
    bool
        True when the container's `env` mapping defines `name`.
    """
    environment = container.get("env")
    return isinstance(environment, dict) and name in environment
