"""End-to-end tests for the `make spelling` gate and the pinned release.

These tests run the pinned `typos-config-builder` and the Typos binary it
packages against a throwaway repository, a fixture overlay and a local
dictionary source. That exercises the written `typos.toml`, a staged
misspelling and an unstaged one without the live dictionary, so a shared
dictionary edit cannot change what the tests assert.

Resolving the pin needs the network on a cold cache, so the tests are skipped
unless `make spelling-integration-test` runs them; that target provisions the
pin first. The behavioural coverage that needs no network lives in
:mod:`tests.test_spelling_gate`.
"""

from __future__ import annotations

import os
import tomllib
import typing as typ

import pytest

from tests.spelling_gate_fixtures import (
    BROAD_COLOUR_PATTERN,
    CORRECTED_PHRASE,
    PROHIBITED_PHRASE,
    ScenarioFactory,
    scenario_factory,
)
from tests.spelling_gate_support import make_spelling, quoted, stage

if typ.TYPE_CHECKING:
    from pathlib import Path

#: Set only by `make spelling-integration-test`, which resolves the pinned
#: builder before running these tests.
INTEGRATION_FLAG = "FEMTOLOGGING_SPELLING_INTEGRATION"

#: The end-to-end tests need the pinned release; the stub tests never do.
requires_pinned_builder = pytest.mark.skipif(
    os.environ.get(INTEGRATION_FLAG) != "1",
    reason="run `make spelling-integration-test` to exercise the pinned builder",
)


@pytest.fixture
def gate_scenario(tmp_path: Path) -> ScenarioFactory:
    """Return a factory that builds a fixture with its own builder.

    Parameters
    ----------
    tmp_path
        Per-test scratch directory.

    Returns
    -------
    ScenarioFactory
        Factory taking the Markdown body to check and whether to stage it.
    """
    return scenario_factory(tmp_path)


@requires_pinned_builder
@pytest.mark.timeout(300)
def test_the_gate_regenerates_configuration_and_reports_a_misspelling(
    gate_scenario: ScenarioFactory,
) -> None:
    """The real gate writes the merged config and fails on a seeded misspelling.

    The overlay word proves the local policy reached the generated file, and
    the withdrawn colour mask proves an overlay `remove` entry is honoured.
    """
    scenario = gate_scenario("# Notes\n\nA mispell is seeded here.\n")

    result = make_spelling(
        scenario.repository, quoted("TYPOS_CONFIG_BUILDER", scenario.builder)
    )

    assert result.returncode == 2, result.stdout + result.stderr
    generated = (scenario.repository / "typos.toml").read_text("utf-8")
    configuration = tomllib.loads(generated)
    accepted = configuration["default"]["extend-words"]
    assert accepted.get("zzgatefixture") == "zzgatefixture", accepted
    # The pattern values are JSON-escaped inside the TOML strings, so a raw
    # substring search would miss them: reading the file back is what proves
    # the overlay's `remove` was honoured rather than merely unrecognised.
    patterns = configuration["default"]["extend-ignore-re"]
    assert "zzgatefixture-sentinel" in patterns, patterns
    assert BROAD_COLOUR_PATTERN not in patterns, patterns
    assert "mispell" in result.stdout + result.stderr, result.stdout + result.stderr


@requires_pinned_builder
@pytest.mark.timeout(300)
def test_the_gate_passes_a_clean_fixture(
    gate_scenario: ScenarioFactory,
) -> None:
    """The same fixture without the misspelling passes and still writes config."""
    scenario = gate_scenario("# Notes\n\nNothing to correct here.\n")

    result = make_spelling(
        scenario.repository, quoted("TYPOS_CONFIG_BUILDER", scenario.builder)
    )

    assert result.returncode == 0, result.stdout + result.stderr
    generated = (scenario.repository / "typos.toml").read_text("utf-8")
    accepted = tomllib.loads(generated)["default"]["extend-words"]
    assert accepted.get("zzgatefixture") == "zzgatefixture", accepted


@requires_pinned_builder
@pytest.mark.timeout(600)
def test_an_unstaged_misspelling_fails_before_and_after_staging(
    gate_scenario: ScenarioFactory,
) -> None:
    """A new document is checked whether or not it has been staged.

    Before staging the gate cannot see the document, so the second stage has to
    catch it; after staging the gate itself does. Both runs must fail, or a
    contributor would learn about the finding only after committing.
    """
    scenario = gate_scenario("# Notes\n\nA mispell is seeded here.\n", staged=False)
    assignments = (quoted("TYPOS_CONFIG_BUILDER", scenario.builder),)

    unstaged = make_spelling(scenario.repository, *assignments)
    assert unstaged.returncode == 2, unstaged.stdout + unstaged.stderr
    assert "mispell" in unstaged.stdout + unstaged.stderr, unstaged.stdout

    stage(scenario.repository, "notes.md")
    staged = make_spelling(scenario.repository, *assignments)
    assert staged.returncode == 2, staged.stdout + staged.stderr
    assert "mispell" in staged.stdout + staged.stderr, staged.stdout


@requires_pinned_builder
@pytest.mark.timeout(600)
def test_a_phrase_the_unstaged_document_carries_fails_before_and_after_staging(
    gate_scenario: ScenarioFactory,
) -> None:
    """A shared phrase correction reaches a new document whether staged or not.

    Typos tokenizes on punctuation, so the hyphenated prohibited phrase is
    invisible to the second stage. Before staging, only the phrase check over
    the throwaway index can catch it; after staging, the gate's own phrase
    check does. Both runs must fail, or a contributor would learn about the
    finding only after committing.
    """
    notes = f"# Notes\n\nA {PROHIBITED_PHRASE} example.\n"
    scenario = gate_scenario(notes, staged=False)
    assignments = (quoted("TYPOS_CONFIG_BUILDER", scenario.builder),)
    finding = f"{PROHIBITED_PHRASE} -> {CORRECTED_PHRASE}"

    unstaged = make_spelling(scenario.repository, *assignments)
    assert unstaged.returncode == 2, unstaged.stdout + unstaged.stderr
    assert finding in unstaged.stdout + unstaged.stderr, (
        unstaged.stdout + unstaged.stderr
    )

    stage(scenario.repository, "notes.md")
    staged = make_spelling(scenario.repository, *assignments)
    assert staged.returncode == 2, staged.stdout + staged.stderr
    assert finding in staged.stdout + staged.stderr, staged.stdout + staged.stderr
