"""Behavioural tests for the `make spelling` recipe's three stages.

`make spelling` runs the pinned `typos-config-builder gate`, which enumerates
the Git index, then checks the Markdown the gate cannot see: files that are new
and still unstaged. Typos cannot express the shared phrase corrections, so the
gate's own phrase check runs again over a temporary index that adds those
files. These tests drive all three stages with stub executables and a scratch
repository, proving the arguments each stage receives, that staging moves a
document from the later stages to the first, that the phrase stage sees the new
document without disturbing the real index, and that a failure in any stage
fails the target.

The end-to-end tests that run the pinned release live in
:mod:`tests.test_spelling_gate_end_to_end`, behind
`make spelling-integration-test`, because resolving the pin needs the network
on a cold cache.
"""

from __future__ import annotations

import subprocess  # ruff: ignore[suspicious-subprocess-import] - fixed argv
import typing as typ

from tests.build_tools_support import make_environment
from tests.spelling_gate_support import (
    make_spelling,
    phrase_index_probe,
    phrase_scan,
    quoted,
    recorded_invocations,
    recording_stub,
    required_tool,
    scratch_repository,
    stage,
)

if typ.TYPE_CHECKING:
    from pathlib import Path


def _untracked(repository: Path) -> list[str]:
    """Return the repository's untracked paths, as Git reports them."""
    result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] - fixed argv
        [required_tool("git"), "status", "--porcelain"],
        cwd=repository,
        capture_output=True,
        text=True,
        check=True,
        env=make_environment(),
    )
    return [line for line in result.stdout.splitlines() if line.startswith("??")]


def test_the_target_runs_the_gate_and_then_the_phrase_check(
    tmp_path: Path,
) -> None:
    """The recipe runs the builder's gate, then its phrase check, once each."""
    repository = scratch_repository(tmp_path)
    builder = recording_stub(tmp_path, "stub-builder", 0)
    typos = recording_stub(tmp_path, "stub-typos", 0)

    result = make_spelling(
        repository,
        quoted("TYPOS_CONFIG_BUILDER", builder),
        quoted("TYPOS", typos),
        phrase_scan(tmp_path),
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert recorded_invocations(tmp_path) == [["gate"], ["check-phrases"]], (
        "the target must pass the gate operation and the phrase check, once each"
    )


def test_a_failing_builder_gate_fails_the_target(tmp_path: Path) -> None:
    """A builder that reports a finding fails `make spelling`."""
    repository = scratch_repository(tmp_path)
    builder = recording_stub(tmp_path, "stub-builder", 2)
    typos = recording_stub(tmp_path, "stub-typos", 0)

    result = make_spelling(
        repository,
        quoted("TYPOS_CONFIG_BUILDER", builder),
        quoted("TYPOS", typos),
        phrase_scan(tmp_path),
    )

    assert result.returncode != 0, "a non-zero builder exit must fail the target"
    assert recorded_invocations(tmp_path) == [["gate"]], (
        "the failing run must stop before the later stages"
    )


def test_an_unstaged_document_reaches_the_second_stage(tmp_path: Path) -> None:
    """New Markdown the gate cannot see is checked by the pinned Typos binary."""
    repository = scratch_repository(tmp_path)
    (repository / "new-notes.md").write_text("# Notes\n", encoding="utf-8")
    builder = recording_stub(tmp_path, "stub-builder", 0)
    typos = recording_stub(tmp_path, "stub-typos", 0)

    result = make_spelling(
        repository,
        quoted("TYPOS_CONFIG_BUILDER", builder),
        quoted("TYPOS", typos),
        phrase_scan(tmp_path),
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert recorded_invocations(tmp_path) == [
        ["gate"],
        ["--", "new-notes.md"],
        ["check-phrases"],
    ], "the unstaged document must be checked after the gate regenerates the config"


def test_a_staged_document_is_left_to_the_gate(tmp_path: Path) -> None:
    """Staging a document moves it into the gate's own scan."""
    repository = scratch_repository(tmp_path)
    (repository / "new-notes.md").write_text("# Notes\n", encoding="utf-8")
    stage(repository, "new-notes.md")
    builder = recording_stub(tmp_path, "stub-builder", 0)
    typos = recording_stub(tmp_path, "stub-typos", 0)

    result = make_spelling(
        repository,
        quoted("TYPOS_CONFIG_BUILDER", builder),
        quoted("TYPOS", typos),
        phrase_scan(tmp_path),
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert recorded_invocations(tmp_path) == [["gate"], ["check-phrases"]], (
        "a staged document must be left to the gate rather than checked twice"
    )


def test_a_failing_unstaged_check_fails_the_target(tmp_path: Path) -> None:
    """A finding in the second stage fails `make spelling`."""
    repository = scratch_repository(tmp_path)
    (repository / "new-notes.md").write_text("# Notes\n", encoding="utf-8")
    builder = recording_stub(tmp_path, "stub-builder", 0)
    typos = recording_stub(tmp_path, "stub-typos", 2)

    result = make_spelling(
        repository,
        quoted("TYPOS_CONFIG_BUILDER", builder),
        quoted("TYPOS", typos),
        phrase_scan(tmp_path),
    )

    assert result.returncode != 0, "a failing unstaged check must fail the target"
    assert recorded_invocations(tmp_path) == [
        ["gate"],
        ["--", "new-notes.md"],
    ], "a failing Typos stage must stop the target before the phrase check"


def test_a_phrase_finding_in_an_unstaged_document_fails_the_target(
    tmp_path: Path,
) -> None:
    """A phrase the second stage cannot see still fails the target."""
    repository = scratch_repository(tmp_path)
    (repository / "new-notes.md").write_text("# Notes\n", encoding="utf-8")
    builder = recording_stub(tmp_path, "stub-builder", 0)
    typos = recording_stub(tmp_path, "stub-typos", 0)

    result = make_spelling(
        repository,
        quoted("TYPOS_CONFIG_BUILDER", builder),
        quoted("TYPOS", typos),
        phrase_scan(tmp_path, status=2),
    )

    assert result.returncode != 0, "a phrase finding must fail the target"
    assert recorded_invocations(tmp_path) == [
        ["gate"],
        ["--", "new-notes.md"],
        ["check-phrases"],
    ], "the phrase stage must run after the Typos stage and report its finding"


def test_the_phrase_stage_reads_the_untracked_document_from_a_temporary_index(
    tmp_path: Path,
) -> None:
    """The phrase check sees the new document and the real index is untouched."""
    repository = scratch_repository(tmp_path)
    (repository / "new-notes.md").write_text("# Notes\n", encoding="utf-8")
    builder = recording_stub(tmp_path, "stub-builder", 0)
    typos = recording_stub(tmp_path, "stub-typos", 0)

    result = make_spelling(
        repository,
        quoted("TYPOS_CONFIG_BUILDER", builder),
        quoted("TYPOS", typos),
        phrase_index_probe(tmp_path),
    )

    assert result.returncode == 0, result.stdout + result.stderr
    assert _untracked(repository) == ["?? new-notes.md"], (
        "the temporary index must not stage the document in the real one"
    )
