"""Rules for the heavy job's `Run formatters and tests` step.

The step is a gate in its own right: it runs the deterministic Rust and Python
lanes the issue forbids weakening. A command can keep its text and still do
none of its work, so every required command must be a whole executed line, and
no line may discard a failure's exit status, pipe or background a gate command,
or sit in a block the shell never enters. The step's compiler environment is
checked too, because an encoded-flags value would drop `--cfg loom` from the
Loom Clippy lane without changing a word of the command.

The document accessors live in :mod:`tests.heavy_workflow_document`.
"""

from __future__ import annotations

import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

from tests.heavy_encoded_flags_contract import encoded_flags_problems
from tests.heavy_workflow_document import (
    FORMATTERS_STEP,
    HEAVY_JOB,
    Job,
    run_text,
    steps,
)

__all__ = ["formatter_command_problems"]

#: Commands the `Run formatters and tests` script must invoke, each forming a
#: whole executed line after line continuations are joined. An exact line, not
#: a prefix: any argument appended to a gate command can change what it runs
#: without changing what it looks like. `pytest -q --collect-only` collects the
#: suite and executes none of it, and `cargo clippy ... -- -D warnings` with a
#: trailing `--cap-lints allow` reports nothing, while both keep the command's
#: text. The issue forbids removing or weakening these lanes, so the separate
#: scheduled Loom all-targets Clippy invocation is required here too: a `|| :`
#: suffix or a `#`-prefixed copy would otherwise pass the pull-request text
#: assertion while no Loom Clippy ran on the scheduled path.
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

#: Constructs that make a command conditional on something false. A command
#: inside one is never executed, so a line-prefix match would be satisfied
#: while the lane never runs. The opener is matched as a whole statement so a
#: multi-line block whose body begins with the required command is caught too.
_NEVER_RUNNING_BLOCKS: tuple[str, ...] = ("if false; then", "if false")


def formatter_command_problems(
    document: cabc.Mapping[str, object], found: Job
) -> list[str]:
    """Return complaints about the formatter-and-test step's script.

    A commented-out command keeps the text a naive substring search would
    match, so each required command must be a whole executed line that is not a
    comment, and no line may mask a failure's exit status. The step's compiler
    environment is checked too: an encoded-flags value outranks the inline
    flags and would drop `--cfg loom` from the Loom Clippy lane without
    changing a word of the command.

    Returns
    -------
    list[str]
        One complaint per missing or commented-out command, one per masking
        fragment, and one per encoded-flags declaration that could displace
        the inline flags.
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
        *_background_problems(executed),
        *encoded_flags_problems(FORMATTERS_STEP, step, found, document),
    ]


def _executed_lines(script: str) -> list[str]:
    """Return the script's logical lines, joined and stripped.

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
    """Return one complaint per required command no executed line matches.

    The whole line must equal the command. A prefix match would accept a
    command with an argument appended, and several such arguments replace the
    work the gate exists to do: `pytest -q --collect-only` executes no test,
    and a trailing `--cap-lints allow` silences the Clippy lane.

    Returns
    -------
    list[str]
        The complaints, in the required-command order.
    """
    return [
        f"the {FORMATTERS_STEP!r} step must run exactly `{command}`"
        for command in _REQUIRED_FORMATTER_COMMANDS
        if command not in executed
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

    The contract matches a required command by its line start, which a command
    nested in a block the shell never enters would still satisfy. The scripts
    here have no legitimate `if false` block, so one is refused outright
    rather than parsed.

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


def _pipes(line: str, command: str) -> bool:
    """Return whether `line` pipes `command`.

    The command may carry a suffix of its own, so the line is matched by
    prefix and the suffix then searched for a pipe. An exact-line match alone
    is not enough here: the finding this rule exists for appends `| tee`, and
    that appended text is exactly the pipe being refused.

    Returns
    -------
    bool
        True when the line starts with the command and pipes it.
    """
    return line.startswith(command) and "|" in line[len(command) :]


def _backgrounds(line: str, command: str) -> bool:
    """Return whether `line` backgrounds `command`.

    As with :func:`_pipes`, the appended `&` is itself the defect, so a suffix
    is searched rather than compared.

    Returns
    -------
    bool
        True when the line starts with the command and backgrounds it.
    """
    return line.startswith(command) and "&" in line[len(command) :]


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
    return [
        f"the {FORMATTERS_STEP!r} step must not pipe `{command}`"
        for command in _REQUIRED_FORMATTER_COMMANDS
        if any(_pipes(line, command) for line in executed)
    ]


def _background_problems(executed: list[str]) -> list[str]:
    """Return a complaint per required command that is backgrounded.

    A `&` anywhere after the command starts it in the background. A bare `&`
    returns zero at once, and `& wait` returns the status of `wait` itself,
    which is zero for an argument-free call even when the background job
    failed. Either way the shell continues and the step succeeds while the gate
    command failed. The scripts here never need to background a gate command.

    Returns
    -------
    list[str]
        One complaint per required command that carries a background `&`.
    """
    return [
        f"the {FORMATTERS_STEP!r} step must not background `{command}`"
        for command in _REQUIRED_FORMATTER_COMMANDS
        if any(_backgrounds(line, command) for line in executed)
    ]
