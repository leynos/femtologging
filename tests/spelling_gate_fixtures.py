"""Fixture repositories and the pinned builder wired to them.

The `make spelling` tests need a scratch repository, a local dictionary
authority and a wrapper that forwards to the pinned release. Building all
three in one place keeps the clean, seeded and unstaged cases on the same gate
configuration. The scratch repository, stubs and Make invocation they use live
in :mod:`tests.spelling_gate_support`.
"""

from __future__ import annotations

import dataclasses as dc
import shlex
import typing as typ

from tests.build_tools_support import fake_executable
from tests.make_contract_helpers import variable_tokens
from tests.spelling_gate_support import scratch_repository, stage

if typ.TYPE_CHECKING:
    from pathlib import Path

#: The prohibited phrase the fixture exercises. The phrase check matches
#: adjacent word tokens wherever they appear, including in this file, so the
#: words are joined through a variable rather than written side by side.
_HYPHEN = "-"
PROHIBITED_PHRASE = "hand" + _HYPHEN + "written"

#: The correction the shared phrase table applies to it.
CORRECTED_PHRASE = "handwritten"

#: A minimal shared dictionary. It carries the broad colour-option mask the
#: overlay withdraws, a sentinel mask it leaves alone, one misspelling the
#: fixture document repeats, and one phrase correction Typos cannot express.
FIXTURE_DICTIONARY = f"""\
schema = 1

[oxford]
stems = []

[words]
accepted = []

[words.corrections]
mispell = "misspell"

[patterns]
ignore = [
    '--[A-Za-z0-9_-]*color(?:ed|s|ing|ize[ds]?|izing)?(?:\\b|_)',
    'zzgatefixture-sentinel',
]

[phrases.corrections]
"{PROHIBITED_PHRASE}" = "{CORRECTED_PHRASE}"

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


@dc.dataclass(frozen=True, slots=True)
class GateScenario:
    """A fixture repository wired to the pinned builder.

    Attributes
    ----------
    repository
        Fixture repository the gate operates on.
    builder
        Wrapper that forwards to the pinned builder for this repository.
    """

    repository: Path
    builder: Path


class ScenarioFactory(typ.Protocol):
    """Build one gate fixture and the builder wired to it from *notes*.

    `notes` is the Markdown body the gate scans; `staged` decides whether the
    gate itself or the recipe's later stages are expected to see the fixture. A
    plain `Callable[[str, bool], GateScenario]` would claim both parameters are
    required and positional, which the keyword-only `staged` default denies.
    """

    def __call__(self, notes: str, *, staged: bool = True) -> GateScenario:
        """Create a fixture repository and its forwarding builder."""


def forwarding_builder(scratch: Path, repository: Path, dictionary: Path) -> Path:
    """Write a builder that runs the pinned release against the fixture.

    The recipe passes `gate` and, when a document is untracked, `check-phrases`.
    Both are forwarded to the pinned command with the gate's own
    `--repository`; `gate` alone also takes the fixture's local dictionary, so
    the shared phrase table it writes into the cache is the one the phrase
    check then reads. `check-phrases` has no source option.

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
    fixture = shlex.quote(str(repository))
    source = shlex.quote(str(dictionary))
    return fake_executable(
        scratch / "forwarding-builder",
        f'if [ "$1" = gate ]; then\n'
        f'  exec {pinned} "$@" --repository={fixture} --source={source}\n'
        f"fi\n"
        f'exec {pinned} "$@" --repository={fixture}\n',
    )


def fixture_repository(
    repository: Path,
    notes: str,
    *,
    staged: bool,
) -> Path:
    """Fill the fixture repository with a document and its policy files.

    Parameters
    ----------
    repository
        Fixture repository root, already initialised.
    notes
        Markdown body the gate scans.
    staged
        Whether to stage the fixture files. An unstaged document is visible
        only to `make spelling`'s own Typos stage.

    Returns
    -------
    Path
        The fixture repository root.
    """
    (repository / "typos.local.toml").write_text(FIXTURE_OVERLAY, encoding="utf-8")
    (repository / "notes.md").write_text(notes, encoding="utf-8")
    (repository / "typos.toml").write_text("", encoding="utf-8")
    if staged:
        stage(repository, "notes.md", "typos.local.toml", "typos.toml")
    return repository


def scenario_factory(scratch: Path) -> ScenarioFactory:
    """Return a factory that builds a fixture with its own builder.

    Building the repository, the local dictionary and the wrapper in one place
    keeps the clean, seeded and unstaged cases on the same gate configuration.

    Parameters
    ----------
    scratch
        Directory receiving the repository, the dictionary and the wrapper.

    Returns
    -------
    ScenarioFactory
        Factory taking the Markdown body to check and whether to stage it.
    """
    dictionary = scratch / "dictionary.toml"
    dictionary.write_text(FIXTURE_DICTIONARY, encoding="utf-8")

    def build(notes: str, *, staged: bool = True) -> GateScenario:
        """Create one fixture repository and its forwarding builder."""
        repository = fixture_repository(
            scratch_repository(scratch), notes, staged=staged
        )
        builder = forwarding_builder(scratch, repository, dictionary)
        return GateScenario(repository=repository, builder=builder)

    return build
