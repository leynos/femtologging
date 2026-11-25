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
    FORMATTERS_STEP,
    HEAVY_JOB,
    LOOM_STEP,
    WORKFLOW,
    Job,
    run_text,
    steps,
)
from tests.make_contract_helpers import command_words, defaults_shell

__all__ = [
    "command_words",
    "loom_condition_problems",
    "loom_tolerance_problems",
    "run_tokens",
    "shell_problems",
]

#: The steps the scheduled job must keep and must not make skippable.
#:
#: `Run formatters and tests` carries the deterministic Rust and Python lanes
#: the issue forbids weakening, so a condition or tolerated failure there is as
#: much a gate failure as one on either Loom step.
GATE_STEPS: tuple[str, ...] = (COMPILE_STEP, LOOM_STEP, FORMATTERS_STEP)

#: Operators that also bind without surrounding whitespace. The shell keeps
#: `&wait` attached to the preceding word, so each token is split on these
#: before it is judged.
_ATTACHED_OPERATORS: typ.Final = re.compile(r"([&;|])")

#: A shell redirection. The operand that follows is not an argument, so a
#: command carrying one cannot be validated as a plain argument vector.
_REDIRECTION: typ.Final = re.compile(r"(?:^|\s)(?:\d*[<>]|&>)")

#: The `cargo test` subcommand arguments the compile step must carry.


def run_tokens(step: dict[str, object]) -> tuple[str, ...]:
    """Return the shell command of a step as operator-separated tokens.

    A shell separates `&`, `;` and `|` from their neighbours, with or without
    whitespace, so the lexical tokens are split again on those characters. A
    command ending in `&wait`, or `true;handler`, then exposes the operator and
    the builtin the foreground rule looks for. An unquoted `#` ends the
    command, so a comment-only script tokenizes to nothing.

    A redirection makes the next word an operand rather than an argument — a
    `> --test` consumes the flag as a filename — so a command containing one
    returns a token no rule accepts.

    Returns
    -------
    tuple[str, ...]
        The step's command tokens, with line continuations flattened and
        attached control operators separated.
    """
    command = run_text(step).strip()
    if _REDIRECTION.search(command.replace("\\\n", " ")):
        return ("<redirect>",)
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
    every named gate step.

    Returns
    -------
    list[str]
        One complaint per scope or step that overrides the shell.
    """
    problems: list[str] = []
    named = {step.get("name"): step for step in steps(found, subject=HEAVY_JOB)}
    for step_name in GATE_STEPS:
        step = named.get(step_name)
        if step is not None and "shell" in step:
            problems.append(f"the {step_name!r} step must not override the shell")
    if defaults_shell(found) is not None:
        problems.append(f"the {HEAVY_JOB!r} job must not set defaults.run.shell")
    if defaults_shell(document) is not None:
        problems.append(f"{WORKFLOW} must not set defaults.run.shell")
    return problems


def loom_condition_problems(found: Job) -> list[str]:
    """Return one complaint per condition on the heavy job or a gate step.

    A condition is never judged by its value: ``if: false`` and a
    prior-success expression skip the gate equally, and the workflow's step
    conditions are evaluated before the step runs. A gate that can be skipped
    while the job reports success is not a gate. `Run formatters and tests`
    carries the deterministic Rust and Python lanes the issue requires be
    kept, so it is covered alongside the two Loom steps.

    Returns
    -------
    list[str]
        A complaint for the job or each named gate step that carries ``if``.
    """
    problems: list[str] = []
    if "if" in found:
        problems.append(f"the {HEAVY_JOB!r} job must carry no condition")
    found_steps = steps(found, subject=HEAVY_JOB)
    problems.extend(
        f"the gate step {step.get('name')!r} must carry no condition"
        for step in found_steps
        if step.get("name") in GATE_STEPS and "if" in step
    )
    return problems


def loom_tolerance_problems(found: Job) -> list[str]:
    """Return one complaint per gate step that tolerates its own failure.

    The heavy job may not tolerate failure at all, and neither may any named
    gate step. Without this rule, ``continue-on-error: true`` on the execution
    step would report a failing model run as a successful job, and the same on
    `Run formatters and tests` would hide a failing Rust or Python lane.

    Returns
    -------
    list[str]
        A complaint for the job or each named gate step that tolerates
        failure.
    """
    problems: list[str] = []
    if found.get("continue-on-error", False) is not False:
        problems.append(f"the {HEAVY_JOB!r} job must not tolerate failure")
    found_steps = steps(found, subject=HEAVY_JOB)
    problems.extend(
        f"the gate step {step.get('name')!r} must not tolerate failure"
        for step in found_steps
        if step.get("name") in GATE_STEPS
        and step.get("continue-on-error", False) is not False
    )
    return problems


#: Commands the `Run formatters and tests` script must invoke, each beginning an
#: executed line after line continuations are joined. The issue forbids
#: removing or weakening these lanes, so the separate scheduled Loom
#: all-targets Clippy invocation is required here too: a `|| :` suffix or a
#: `#`-prefixed copy would otherwise pass the pull-request text assertion while
#: no Loom Clippy ran on the scheduled path.
_REQUIRED_FORMATTER_COMMANDS: tuple[str, ...] = (
    "cargo fmt --manifest-path rust_extension/Cargo.toml -- --check",
    "make lint-rust-clippy",
    (
        'RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold" cargo clippy '
        "--manifest-path rust_extension/Cargo.toml --no-default-features "
        "--all-targets -- -D warnings"
    ),
    "uv run --locked pytest -q",
)

#: Fragments that mask a failed command's status anywhere in the script. `||`
#: is refused outright rather than only `|| true`, because `|| :` and any other
#: no-op right-hand side discards the left-hand status just as effectively. A
#: bare `|` is refused too: GitHub's default `bash -e {0}` shell does not set
#: `pipefail`, so `clippy | true` reports the right-hand status and hides a
#: Clippy failure.
_FORMATTER_MASK_FRAGMENTS: tuple[str, ...] = (
    "||",
    "set +e",
    "exit 0",
)

#: Any pipe on a required command is refused. GitHub's default `bash -e {0}`
#: shell does not set `pipefail`, so *any* right-hand side, not only `true` or
#: `cat`, can report success while the gate command failed.

#: Constructs that make a command conditional on something false. A command
#: inside one is never executed, so a line-prefix match would be satisfied
#: while the lane never runs. The opener is matched as a whole statement so a
#: multi-line block whose body begins with the required command is caught too.
_NEVER_RUNNING_BLOCKS: tuple[str, ...] = ("if false; then", "if false")


def formatter_command_problems(found: Job) -> list[str]:
    """Return complaints about the formatter-and-test step's script.

    The step is a gate in its own right: it runs the deterministic Rust
    formatting, Clippy and pytest lanes, and the issue forbids removing or
    weakening them. A commented-out command keeps the text a naive substring
    search would match, so each required command must begin a line that is not
    a comment, and no line may mask a failure's exit status.

    Returns
    -------
    list[str]
        One complaint per missing or commented-out command, and one per masking
        fragment.
    """
    step = next(
        (
            step
            for step in steps(found, subject=HEAVY_JOB)
            if step.get("name") == FORMATTERS_STEP
        ),
        None,
    )
    if step is None:
        return [f"the job must have a {FORMATTERS_STEP!r} step"]
    executed = _executed_lines(run_text(step))
    return [
        *_conditional_skip_problems(executed),
        *_missing_formatter_command_problems(executed),
        *_formatter_mask_problems(executed),
        *_pipeline_problems(executed),
    ]


def _executed_lines(script: str) -> list[str]:
    """Return the script's logicssal lines, joined and stripped.

    Backslash-newline continuations are joined first, so a command wrapped
    across lines is one entry; whitespace is then collapsed, because the shell
    treats any run of it as one separator. The script's own trailing `#`
    comments are dropped as a shell would drop them.

    Returns
    -------
    list[str]
        The lines the shell would actually run, without comments.
    """
    joined = script.replace("\\\n", " ")
    return [
        " ".join(code.split())
        for line in joined.splitlines()
        if (code := _strip_comment(line)) and not code.startswith("#")
    ]


def _strip_comment(line: str) -> str:
    """Return `line` without a trailing shell comment, stripped.

    A `#` only begins a comment at the start of a word, so `${{ ... }}` and a
    quoted `#` are left alone; the scripts here never quote one.

    Returns
    -------
    str
        The code part of the line.
    """
    code = line.split(" #", 1)[0]
    return code.strip()


def _missing_formatter_command_problems(executed: list[str]) -> list[str]:
    """Return one complaint per required command no executed line begins.

    Returns
    -------
    list[str]
        The complaints, in the required-command order.
    """
    return [
        f"the {FORMATTERS_STEP!r} step must run `{command}`"
        for command in _REQUIRED_FORMATTER_COMMANDS
        if not any(line.startswith(command) for line in executed)
    ]


def _formatter_mask_problems(executed: list[str]) -> list[str]:
    """Return one complaint per masking fragment an executed line contains.

    Returns
    -------
    list[str]
        The complaints, in the fragment order.
    """
    return [
        f"the {FORMATTERS_STEP!r} step must not mask a failure with {fragment!r}"
        for fragment in _FORMATTER_MASK_FRAGMENTS
        if any(fragment in line for line in executed)
    ]


def _conditional_skip_problems(executed: list[str]) -> list[str]:
    """Return a complaint when a command sits in an `if false` block.

    The contract matches a required command by its line prefix, which a
    command nested in a block the shell never enters would still satisfy. The
    scripts here have no legitimate `if false` block, so one is refused
    outright rather than parsed.

    Returns
    -------
    list[str]
        A complaint when an `if false` block appears in the script.
    """
    return [
        f"the {FORMATTERS_STEP!r} step must not wrap a command in `{opener} ... fi`"
        for opener in _NEVER_RUNNING_BLOCKS
        if any(line.startswith(opener) for line in executed)
    ]


def _pipeline_problems(executed: list[str]) -> list[str]:
    """Return a complaint per required command that is piped.

    Without `pipefail`, a pipeline reports its last command's status, so a
    suffix as innocuous as `| tee /dev/null` hides a failure. The scripts here
    never need to pipe a gate command, so any pipe on a required command is
    refused rather than judged for its right-hand side.

    Returns
    -------
    list[str]
        One complaint per required command that is piped.
    """
    problems: list[str] = []
    for command in _REQUIRED_FORMATTER_COMMANDS:
        for line in executed:
            if line.startswith(command) and "|" in line[len(command) :]:
                problems.append(
                    f"the {FORMATTERS_STEP!r} step must not pipe `{command}`"
                )
                break
    return problems
