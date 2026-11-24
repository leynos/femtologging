"""Deliberate breaks of the heavy job's Loom compile gate.

Each function mutates a deep copy of the parsed workflow in one way the
compile-gate contract must report. They are separate from the other heavy
mutations to keep both modules below the repository's line limit.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.heavy_workflow_document import (
    COMPILE_STEP,
    HEAVY_JOB,
    job,
    named_step,
    raw_steps,
    run_text,
)

type Document = dict[str, typ.Any]


def _compile_step_conditioned(document: Document) -> None:
    """Make the Loom compile step skippable."""
    found = job(document, HEAVY_JOB)
    named_step(found, COMPILE_STEP, subject=HEAVY_JOB)["if"] = "false"


def _compile_step_tolerant(document: Document) -> None:
    """Let a failed Loom compile stop failing the job."""
    found = job(document, HEAVY_JOB)
    named_step(found, COMPILE_STEP, subject=HEAVY_JOB)["continue-on-error"] = True


def _compile_step_removed(document: Document) -> None:
    """Delete the separate compile gate."""
    found = job(document, HEAVY_JOB)
    found_steps = raw_steps(found, subject=HEAVY_JOB)
    found_steps.remove(named_step(found, COMPILE_STEP, subject=HEAVY_JOB))


def _compile_step_echoes(document: Document) -> None:
    """Keep the compile step's text but only print it."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, COMPILE_STEP, subject=HEAVY_JOB)
    step["run"] = f"echo '{run_text(step)}'"


def _compile_step_shell_never_runs(document: Document) -> None:
    """Ask GitHub to parse the compile step without executing it."""
    found = job(document, HEAVY_JOB)
    named_step(found, COMPILE_STEP, subject=HEAVY_JOB)["shell"] = "bash -n {0}"


def _compile_step_asks_for_help(document: Document) -> None:
    """Make Cargo print help instead of compiling."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, COMPILE_STEP, subject=HEAVY_JOB)
    step["run"] = f"{run_text(step).rstrip()} --help"


def _compile_step_masks_failure(document: Document) -> None:
    """Keep the compile command but discard its exit status."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, COMPILE_STEP, subject=HEAVY_JOB)
    step["run"] = f"{run_text(step).rstrip()} || true"


def _compile_flags_after_separator(document: Document) -> None:
    """Move the target and `--no-run` past Cargo's own `--` separator."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, COMPILE_STEP, subject=HEAVY_JOB)
    step["run"] = (
        'RUSTFLAGS="--cfg loom -C link-arg=-fuse-ld=mold" cargo test --release '
        "--manifest-path rust_extension/Cargo.toml --no-default-features "
        "-- --test heavy --no-run"
    )


def _compile_step_without_loom(document: Document) -> None:
    """Strip ``--cfg loom`` from the compile step's inline flags."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, COMPILE_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace("--cfg loom ", "")


def _compile_encoded_flags_assignment(document: Document) -> None:
    """Displace the inline flags with a command-local encoded value."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, COMPILE_STEP, subject=HEAVY_JOB)
    step["run"] = f"CARGO_ENCODED_RUSTFLAGS=-Cdebuginfo=0 {run_text(step)}"


def _compile_loom_variant(document: Document) -> None:
    """Enable a look-alike configuration instead of `loom`."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, COMPILE_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace("--cfg loom ", "--cfg loom_extra ")


def _compile_split_assignment(document: Document) -> None:
    """Put the assignment on its own line, where the shell will not export it."""
    found = job(document, HEAVY_JOB)
    step = named_step(found, COMPILE_STEP, subject=HEAVY_JOB)
    step["run"] = run_text(step).replace(" cargo test", "\ncargo test", 1)


COMPILE_MUTATIONS = (
    pytest.param(_compile_step_conditioned, id="compile-step-conditioned"),
    pytest.param(_compile_step_tolerant, id="compile-step-tolerant"),
    pytest.param(_compile_step_removed, id="compile-step-removed"),
    pytest.param(_compile_step_without_loom, id="compile-step-without-loom"),
    pytest.param(
        _compile_encoded_flags_assignment, id="compile-encoded-flags-assignment"
    ),
    pytest.param(_compile_loom_variant, id="compile-loom-variant"),
    pytest.param(_compile_split_assignment, id="compile-split-assignment"),
    pytest.param(_compile_step_echoes, id="compile-step-echoes"),
    pytest.param(_compile_step_asks_for_help, id="compile-step-asks-for-help"),
    pytest.param(_compile_step_masks_failure, id="compile-step-masks-failure"),
    pytest.param(_compile_flags_after_separator, id="compile-flags-after-separator"),
    pytest.param(_compile_step_shell_never_runs, id="compile-step-shell-never-runs"),
)
