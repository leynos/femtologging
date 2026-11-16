"""Behavioural and end-to-end tests for the `make spelling` builder gate.

`make spelling` delegates generation and checking to the pinned
`typos-config-builder gate` command, so the recipe is the whole spelling
workflow. The first two tests drive that recipe with a stub builder: one proves
the target passes exactly the `gate` operation, the other proves a failing gate
fails the target. The remaining tests run the pinned builder itself against a
throwaway repository, a fixture overlay and a local dictionary source, so the
written `typos.toml` and the seeded-misspelling failure are checked without the
live dictionary.

Only fetching the pinned builder package reaches the network here. The
dictionary authority is a local file, and the fixture repository keeps its own
policy, so a shared dictionary edit cannot change what these tests assert.
"""

from __future__ import annotations

import shlex
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] - fixed argv
import typing as typ

import pytest

from tests.build_tools_support import ROOT, fake_executable, make_environment
from tests.make_contract_helpers import variable_tokens

if typ.TYPE_CHECKING:
    from pathlib import Path

#: A minimal shared dictionary. It carries the broad colour-option mask the
#: overlay withdraws, and one misspelling the fixture document repeats.
FIXTURE_DICTIONARY = """\
schema = 1

[oxford]
stems = []

[words]
accepted = []

[words.corrections]
mispell = "misspell"

[patterns]
ignore = ['--[A-Za-z0-9_-]*color(?:ed|s|ing|ize[ds]?|izing)?(?:\\b|_)']

[phrases.corrections]

[files]
exclude = []
"""

#: The overlay accepts one fixture word and withdraws the colour mask.
FIXTURE_OVERLAY = """\
schema = 1

[words]
accepted = ["zzgatefixture"]

[patterns]
remove = ['--[A-Za-z0-9_-]*color(?:ed|s|ing|ize[ds]?|izing)?(?:\\b|_)']
"""

#: The exact shared expression the overlay withdraws.
BROAD_COLOUR_PATTERN = r"--[A-Za-z0-9_-]*color(?:ed|s|ing|ize[ds]?|izing)?(?:\b|_)"


def _make_spelling(*assignments: str) -> subprocess.CompletedProcess[str]:
    """Run `make spelling` with command-line variable assignments.

    Parameters
    ----------
    *assignments
        `NAME=value` assignments, which Make places above every `?=` default.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The finished `make` process with its captured output.
    """
    make = shutil.which("make")
    assert make, "make must be on PATH"
    return subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] - fixed argv
        [make, "-C", str(ROOT), "spelling", *assignments],
        capture_output=True,
        text=True,
        check=False,
        env=make_environment(),
    )


def _stub_builder(scratch: Path, status: int) -> Path:
    """Write a builder stub that records its arguments and exits with *status*.

    Parameters
    ----------
    scratch
        Directory receiving the stub and its argument record.
    status
        Exit status the stub reports.

    Returns
    -------
    Path
        The stub executable.
    """
    recorder = scratch / "builder-arguments.txt"
    return fake_executable(
        scratch / "stub-builder",
        f'printf "%s\\n" "$@" > {shlex.quote(str(recorder))}\nexit {status}',
    )


def _recorded_arguments(scratch: Path) -> list[str]:
    """Return the arguments a stub builder received.

    Parameters
    ----------
    scratch
        Directory holding the stub's argument record.

    Returns
    -------
    list[str]
        The recorded arguments, in order.
    """
    record = scratch / "builder-arguments.txt"
    return record.read_text(encoding="utf-8").split()


def _forwarding_builder(scratch: Path, repository: Path, dictionary: Path) -> Path:
    """Write a builder that runs the pinned release against the fixture.

    The Makefile recipe passes exactly one argument, `gate`. Forwarding it to
    the pinned command with the gate's own `--repository` and `--source`
    options exercises the real gate end to end without changing the recipe.

    Parameters
    ----------
    scratch
        Directory receiving the wrapper script.
    repository
        Fixture repository the gate should operate on.
    dictionary
        Local dictionary authority for the gate.

    Returns
    -------
    Path
        The wrapper executable.
    """
    pinned = shlex.join(variable_tokens("TYPOS_CONFIG_BUILDER"))
    return fake_executable(
        scratch / "forwarding-builder",
        f'exec {pinned} "$@"'
        f" --repository={shlex.quote(str(repository))}"
        f" --source={shlex.quote(str(dictionary))}",
    )


def _fixture_repository(scratch: Path, notes: str) -> Path:
    """Create a committed fixture repository holding *notes*.

    Parameters
    ----------
    scratch
        Directory receiving the repository.
    notes
        Markdown body the gate scans.

    Returns
    -------
    Path
        The fixture repository root.
    """
    repository = scratch / "repository"
    repository.mkdir()
    (repository / "typos.local.toml").write_text(FIXTURE_OVERLAY, encoding="utf-8")
    (repository / "notes.md").write_text(notes, encoding="utf-8")
    (repository / "typos.toml").write_text("", encoding="utf-8")
    environment = make_environment(
        GIT_AUTHOR_NAME="Spelling Fixture",
        GIT_AUTHOR_EMAIL="spelling@example.invalid",
        GIT_COMMITTER_NAME="Spelling Fixture",
        GIT_COMMITTER_EMAIL="spelling@example.invalid",
    )
    git = shutil.which("git")
    assert git, "git must be on PATH"
    for command in (
        ["init", "--quiet", "--initial-branch=main", str(repository)],
        ["add", "--", "notes.md", "typos.local.toml", "typos.toml"],
        ["commit", "--quiet", "--message", "fixture"],
    ):
        subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] - fixed argv
            [git, *command],
            cwd=repository,
            capture_output=True,
            text=True,
            check=True,
            env=environment,
        )
    return repository


def test_the_target_passes_only_the_gate_operation_to_the_builder(
    tmp_path: Path,
) -> None:
    """The recipe runs the builder once, with `gate` and nothing else."""
    stub = _stub_builder(tmp_path, 0)
    result = _make_spelling(f"TYPOS_CONFIG_BUILDER={stub}")
    assert result.returncode == 0, result.stdout + result.stderr
    assert _recorded_arguments(tmp_path) == ["gate"], (
        "the target must pass the gate operation and no other argument"
    )


def test_a_failing_builder_gate_fails_the_target(tmp_path: Path) -> None:
    """A builder that reports a finding fails `make spelling`."""
    stub = _stub_builder(tmp_path, 2)
    result = _make_spelling(f"TYPOS_CONFIG_BUILDER={stub}")
    assert result.returncode != 0, "a non-zero builder exit must fail the target"
    assert _recorded_arguments(tmp_path) == ["gate"], (
        "the failing run must still pass only the gate operation"
    )


@pytest.mark.timeout(180)
def test_the_gate_regenerates_configuration_and_reports_a_misspelling(
    tmp_path: Path,
) -> None:
    """The real gate writes the merged config and fails on a seeded misspelling.

    The overlay word proves the local policy reached the generated file, and
    the withdrawn colour mask proves an overlay `remove` entry is honoured.
    """
    dictionary = tmp_path / "dictionary.toml"
    dictionary.write_text(FIXTURE_DICTIONARY, encoding="utf-8")
    repository = _fixture_repository(tmp_path, "# Notes\n\nA mispell is seeded here.\n")
    builder = _forwarding_builder(tmp_path, repository, dictionary)

    result = _make_spelling(f"TYPOS_CONFIG_BUILDER={builder}")

    assert result.returncode == 2, result.stdout + result.stderr
    generated = (repository / "typos.toml").read_text(encoding="utf-8")
    assert '"zzgatefixture" = "zzgatefixture"' in generated, generated
    assert BROAD_COLOUR_PATTERN not in generated, generated
    assert "mispell" in result.stdout + result.stderr, result.stdout + result.stderr


@pytest.mark.timeout(180)
def test_the_gate_passes_a_clean_fixture(tmp_path: Path) -> None:
    """The same fixture without the misspelling passes and still writes config."""
    dictionary = tmp_path / "dictionary.toml"
    dictionary.write_text(FIXTURE_DICTIONARY, encoding="utf-8")
    repository = _fixture_repository(tmp_path, "# Notes\n\nNothing to correct here.\n")
    builder = _forwarding_builder(tmp_path, repository, dictionary)

    result = _make_spelling(f"TYPOS_CONFIG_BUILDER={builder}")

    assert result.returncode == 0, result.stdout + result.stderr
    generated = (repository / "typos.toml").read_text(encoding="utf-8")
    assert '"zzgatefixture" = "zzgatefixture"' in generated, generated
