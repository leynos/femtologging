"""Fixtures and helpers shared by the `make spelling` gate tests.

`make spelling` runs the pinned `typos-config-builder gate` and then checks the
Markdown the gate cannot see: files that are new and still unstaged. The tests
that hold that recipe need the same scratch repository, stub executables, Make
variable assignments and pinned-release wrapper, so they are built here once
rather than copied into each module.

The stub helpers record every invocation as NUL-separated arguments, so an
empty argument stays distinguishable from a missing one and a second invocation
cannot overwrite the first record.
"""

from __future__ import annotations

import dataclasses as dc
import shlex
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] - fixed argv
import typing as typ

from tests.build_tools_support import ROOT, fake_executable, make_environment
from tests.make_contract_helpers import variable_tokens

if typ.TYPE_CHECKING:
    from pathlib import Path

#: A minimal shared dictionary. It carries the broad colour-option mask the
#: overlay withdraws, a sentinel mask it leaves alone, and one misspelling the
#: fixture document repeats.
FIXTURE_DICTIONARY = """\
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

#: File the stubs append their records to. One file serves both stages, so the
#: recorded order is the order the recipe ran them in.
INVOCATION_RECORD = "invocations.txt"

#: Records every invocation as NUL-separated arguments on its own line, so an
#: empty argument stays distinguishable from a missing one.
RECORD_ARGUMENTS = """\
for argument in "$@"; do printf '%s\\000' "$argument" >> {record}; done
printf '\\n' >> {record}
"""


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
    gate itself or the recipe's second stage is expected to see the fixture. A
    plain `Callable[[str, bool], GateScenario]` would claim both parameters are
    required and positional, which the keyword-only `staged` default denies.
    """

    def __call__(self, notes: str, *, staged: bool = True) -> GateScenario:
        """Create a fixture repository and its forwarding builder."""


def recorded_invocations(scratch: Path) -> list[list[str]]:
    """Return the arguments of each stub invocation, in order.

    Parameters
    ----------
    scratch
        Directory holding the stub's argument record.

    Returns
    -------
    list[list[str]]
        One argument list per invocation. An empty argument is preserved as an
        empty string rather than dropped.
    """
    record = scratch / INVOCATION_RECORD
    if not record.exists():
        return []
    return [line.split("\0")[:-1] for line in record.read_text("utf-8").splitlines()]


def recording_stub(scratch: Path, name: str, status: int) -> Path:
    """Write an executable that records its arguments and exits with *status*.

    Parameters
    ----------
    scratch
        Directory receiving the stub and its argument record.
    name
        File name of the stub executable.
    status
        Exit status the stub reports.

    Returns
    -------
    Path
        The stub executable.
    """
    record = shlex.quote(str(scratch / INVOCATION_RECORD))
    body = RECORD_ARGUMENTS.format(record=record) + f"exit {status}"
    return fake_executable(scratch / name, body)


def required_tool(name: str) -> str:
    """Return *name*'s absolute path, failing when the host lacks it.

    Parameters
    ----------
    name
        Executable the fixture recipe needs.

    Returns
    -------
    str
        The resolved executable.

    Raises
    ------
    AssertionError
        If the executable is not on ``PATH``.
    """
    executable = shutil.which(name)
    if executable is None:
        message = f"{name} must be available to run the spelling gate tests"
        raise AssertionError(message)
    return executable


def quoted(name: str, executable: Path) -> str:
    """Return a Make assignment whose value survives the recipe's shell.

    Parameters
    ----------
    name
        Make variable to assign.
    executable
        Path the variable should name.

    Returns
    -------
    str
        The assignment, with the path quoted for the recipe's shell.
    """
    return f"{name}={shlex.quote(str(executable))}"


def shared_environment() -> str:
    """Return the uv cache assignment that keeps a fixture run warm.

    A fixture runs Make from its own directory, so Make's default cache
    locations would resolve inside the fixture and re-resolve the pin. Naming
    the repository's own cache keeps the provisioned release in use.

    Returns
    -------
    str
        A `UV_ENV` assignment pointing at the repository's caches.
    """
    return f"UV_ENV=UV_CACHE_DIR={ROOT / '.uv-cache'} UV_TOOL_DIR={ROOT / '.uv-tools'}"


def make_spelling(
    repository: Path,
    *assignments: str,
) -> subprocess.CompletedProcess[str]:
    """Run `make spelling` against *repository* with variable assignments.

    The repository's own Makefile is not used: the checked-in one is named with
    `-f`, so a fixture repository needs no copy of the build interface. Every
    executable assignment must already be quoted with :func:`quoted`, because
    Make inserts a variable's value into the recipe unquoted and the shell
    would split a path containing spaces.

    Parameters
    ----------
    repository
        Directory Make runs in.
    *assignments
        `NAME=value` assignments, which Make places above every `?=` default.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The finished `make` process with its captured output.
    """
    make = required_tool("make")
    return subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] - fixed argv
        [
            make,
            "-C",
            str(repository),
            "-f",
            str(ROOT / "Makefile"),
            "spelling",
            *assignments,
            shared_environment(),
        ],
        capture_output=True,
        text=True,
        check=False,
        env=make_environment(),
    )


def scratch_repository(scratch: Path) -> Path:
    """Create an empty Git repository for a Make run.

    `make spelling` enumerates the index with `git ls-files`, so even the stub
    tests need a repository to run in rather than the checked-out worktree.

    Parameters
    ----------
    scratch
        Directory receiving the repository.

    Returns
    -------
    Path
        The repository root.
    """
    repository = scratch / "repository"
    repository.mkdir()
    git = required_tool("git")
    subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] - fixed argv
        [git, "init", "--quiet", "--initial-branch=main", str(repository)],
        capture_output=True,
        text=True,
        check=True,
        env=make_environment(),
    )
    return repository


def stage(repository: Path, *names: str) -> None:
    """Add *names* to the index without committing them.

    The gate reads the index, so staging is what makes a file eligible; a
    commit would also require an identity and could fail under commit signing.

    Parameters
    ----------
    repository
        Repository holding the files.
    *names
        Repository-relative file names to stage.
    """
    git = required_tool("git")
    subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] - fixed argv
        [git, "add", "--", *names],
        cwd=repository,
        capture_output=True,
        text=True,
        check=True,
        env=make_environment(),
    )


def forwarding_builder(scratch: Path, repository: Path, dictionary: Path) -> Path:
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
