"""Deliberate breaks of the heavy job's formatter-and-test gate.

Each function mutates a deep copy of the parsed workflow in one way the
`Run formatters and tests` contract must report: a skipped step, a tolerated
failure, a commented-out command, or a masked status. Keeping them separate
keeps every module below the repository's line limit.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.heavy_workflow_document import (
    ENCODED_FLAGS,
    FORMATTERS_STEP,
    HEAVY_JOB,
    env,
    job,
    named_step,
    run_text,
)

type Document = dict[str, typ.Any]


def _formatter_step_conditioned(document: Document) -> None:
    """Skip the deterministic Rust and Python lanes."""
    found = job(document, HEAVY_JOB)
    named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)["if"] = "false"


def _formatter_step_tolerant(document: Document) -> None:
    """Let a failing formatter/test lane leave the job green."""
    found = job(document, HEAVY_JOB)
    named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)["continue-on-error"] = True


def _formatter_step_command_commented(document: Document) -> None:
    """Comment out the Clippy lane while keeping its text."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        "make lint-rust-clippy", "# make lint-rust-clippy"
    )


def _formatter_step_command_masked(document: Document) -> None:
    """Keep the pytest lane but discard its exit status."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        "uv run --locked pytest -q", "uv run --locked pytest -q || true"
    )


def _loom_clippy_commented(document: Document) -> None:
    """Comment out the scheduled Loom Clippy invocation."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        'RUSTFLAGS="--cfg loom', '# RUSTFLAGS="--cfg loom'
    )


def _loom_clippy_masked(document: Document) -> None:
    """Keep the Loom Clippy invocation but discard its exit status."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        "--all-targets -- -D warnings", "--all-targets -- -D warnings || :"
    )


def _loom_clippy_inside_if_false(document: Document) -> None:
    """Wrap the Loom Clippy invocation in a block the shell never enters."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = (
        run_text(step)
        .replace(
            'RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold" cargo clippy',
            "if false; then\n"
            'RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold" cargo clippy',
        )
        .replace("--all-targets -- -D warnings", "--all-targets -- -D warnings; fi")
    )


def _loom_clippy_backgrounded_and_waited(document: Document) -> None:
    """Background the Clippy invocation and reap it with a status-losing `wait`."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        "--all-targets -- -D warnings", "--all-targets -- -D warnings & wait"
    )


def _loom_clippy_backgrounded(document: Document) -> None:
    """Background the Loom Clippy invocation, so the shell returns at once."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        "--all-targets -- -D warnings", "--all-targets -- -D warnings &"
    )


def _loom_clippy_piped_to_tee(document: Document) -> None:
    """Pipe the Loom Clippy invocation into `tee`, which reports its own status."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        "--all-targets -- -D warnings", "--all-targets -- -D warnings | tee /dev/null"
    )


def _loom_clippy_piped_to_true(document: Document) -> None:
    """Pipe the Loom Clippy invocation into `true` without `pipefail`."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        "--all-targets -- -D warnings", "--all-targets -- -D warnings | true"
    )


def _pytest_collects_without_running(document: Document) -> None:
    """Ask pytest to collect the suite without executing any of it."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        "uv run --locked pytest -q", "uv run --locked pytest -q --collect-only"
    )


def _loom_clippy_caps_lints(document: Document) -> None:
    """Append `--cap-lints allow`, which silences every Clippy lint."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(
        "--all-targets -- -D warnings", "--all-targets -- -D warnings --cap-lints allow"
    )


def _formatter_step_encoded_flags(document: Document) -> None:
    """Displace the inline Loom flags with an encoded value on the step."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, FORMATTERS_STEP, subject=HEAVY_JOB)
    environment = env(step)
    environment[ENCODED_FLAGS] = "-Cdebuginfo=0"


FORMATTER_MUTATIONS = (
    pytest.param(_formatter_step_conditioned, id="formatter-step-conditioned"),
    pytest.param(_formatter_step_tolerant, id="formatter-step-tolerant"),
    pytest.param(
        _formatter_step_command_commented,
        id="formatter-step-command-commented",
    ),
    pytest.param(
        _formatter_step_command_masked,
        id="formatter-step-command-masked",
    ),
    pytest.param(_loom_clippy_commented, id="loom-clippy-commented"),
    pytest.param(_loom_clippy_masked, id="loom-clippy-masked"),
    pytest.param(_loom_clippy_inside_if_false, id="loom-clippy-if-false"),
    pytest.param(_loom_clippy_piped_to_true, id="loom-clippy-piped-truthy"),
    pytest.param(_loom_clippy_piped_to_tee, id="loom-clippy-piped-tee"),
    pytest.param(_loom_clippy_backgrounded, id="loom-clippy-backgrounded"),
    pytest.param(
        _loom_clippy_backgrounded_and_waited,
        id="loom-clippy-backgrounded-wait",
    ),
    pytest.param(_pytest_collects_without_running, id="pytest-collect-only"),
    pytest.param(_loom_clippy_caps_lints, id="loom-clippy-cap-lints-allow"),
    pytest.param(_formatter_step_encoded_flags, id="formatter-step-encoded-flags"),
)
