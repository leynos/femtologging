"""Behavioural tests for the `make spelling` recipe's two stages.

`make spelling` runs the pinned `typos-config-builder gate`, which enumerates
the Git index, and then checks the Markdown the gate cannot see: files that are
new and still unstaged. These tests drive both stages with stub executables and
a scratch repository, proving the arguments each stage receives, that staging
moves a document from the second stage to the first, and that a failure in
either stage fails the target.

The end-to-end tests that run the pinned release live in
:mod:`tests.test_spelling_gate_end_to_end`, behind
`make spelling-integration-test`, because resolving the pin needs the network
on a cold cache.
"""

from __future__ import annotations

import typing as typ

from tests.spelling_gate_support import (
    make_spelling,
    quoted,
    recorded_invocations,
    recording_stub,
    scratch_repository,
    stage,
)

if typ.TYPE_CHECKING:
    from pathlib import Path


def test_the_target_passes_only_the_gate_operation_to_the_builder(
    tmp_path: Path,
) -> None:
    """The recipe runs the builder once, with `gate` and nothing else."""
    repository = scratch_repository(tmp_path)
    builder = recording_stub(tmp_path, "stub-builder", 0)
    typos = recording_stub(tmp_path, "stub-typos", 0)

    result = make_spelling(
        repository, quoted("TYPOS_CONFIG_BUILDER", builder), quoted("TYPOS", typos)
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert recorded_invocations(tmp_path) == [["gate"]], (
        "the target must pass the gate operation, once, and no other argument"
    )


def test_a_failing_builder_gate_fails_the_target(tmp_path: Path) -> None:
    """A builder that reports a finding fails `make spelling`."""
    repository = scratch_repository(tmp_path)
    builder = recording_stub(tmp_path, "stub-builder", 2)
    typos = recording_stub(tmp_path, "stub-typos", 0)

    result = make_spelling(
        repository, quoted("TYPOS_CONFIG_BUILDER", builder), quoted("TYPOS", typos)
    )

    assert result.returncode != 0, "a non-zero builder exit must fail the target"
    assert recorded_invocations(tmp_path) == [["gate"]], (
        "the failing run must still pass only the gate operation"
    )


def test_an_unstaged_document_reaches_the_second_stage(tmp_path: Path) -> None:
    """New Markdown the gate cannot see is checked by the pinned Typos binary."""
    repository = scratch_repository(tmp_path)
    (repository / "new-notes.md").write_text("# Notes\n", encoding="utf-8")
    builder = recording_stub(tmp_path, "stub-builder", 0)
    typos = recording_stub(tmp_path, "stub-typos", 0)

    result = make_spelling(
        repository, quoted("TYPOS_CONFIG_BUILDER", builder), quoted("TYPOS", typos)
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert recorded_invocations(tmp_path) == [
        ["gate"],
        ["--", "new-notes.md"],
    ], "the unstaged document must be checked after the gate regenerates the config"


def test_a_staged_document_is_left_to_the_gate(tmp_path: Path) -> None:
    """Staging a document moves it into the gate's own scan."""
    repository = scratch_repository(tmp_path)
    (repository / "new-notes.md").write_text("# Notes\n", encoding="utf-8")
    stage(repository, "new-notes.md")
    builder = recording_stub(tmp_path, "stub-builder", 0)
    typos = recording_stub(tmp_path, "stub-typos", 0)

    result = make_spelling(
        repository, quoted("TYPOS_CONFIG_BUILDER", builder), quoted("TYPOS", typos)
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert recorded_invocations(tmp_path) == [["gate"]], (
        "a staged document must be left to the gate rather than checked twice"
    )


def test_a_failing_unstaged_check_fails_the_target(tmp_path: Path) -> None:
    """A finding in the second stage fails `make spelling`."""
    repository = scratch_repository(tmp_path)
    (repository / "new-notes.md").write_text("# Notes\n", encoding="utf-8")
    builder = recording_stub(tmp_path, "stub-builder", 0)
    typos = recording_stub(tmp_path, "stub-typos", 2)

    result = make_spelling(
        repository, quoted("TYPOS_CONFIG_BUILDER", builder), quoted("TYPOS", typos)
    )

    assert result.returncode != 0, "a failing unstaged check must fail the target"
