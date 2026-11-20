"""Stub executables and Make invocations for the `make spelling` gate tests.

`make spelling` runs the pinned `typos-config-builder gate`, then checks the
Markdown the gate cannot see: files that are new and still unstaged. Typos
cannot express the shared phrase corrections, so the gate's phrase check runs
again over a throwaway index that holds those files. These helpers build the
scratch repository, stub executables and Make variable assignments that drive
that recipe; the dictionary fixtures and pinned-release wrapper they are paired
with live in :mod:`tests.spelling_gate_fixtures`.

The stubs record every invocation as NUL-separated arguments, so an empty
argument stays distinguishable from a missing one and a second invocation
cannot overwrite the first record.
"""

from __future__ import annotations

import shlex
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] - fixed argv
import typing as typ

from tests.build_tools_support import ROOT, fake_executable, make_environment

if typ.TYPE_CHECKING:
    from pathlib import Path

#: File the stubs append their records to. One file serves every stage, so the
#: recorded order is the order the recipe ran them in.
INVOCATION_RECORD = "invocations.txt"

#: Records every invocation as NUL-separated arguments on its own line, so an
#: empty argument stays distinguishable from a missing one.
RECORD_ARGUMENTS = """\
for argument in "$@"; do printf '%s\\000' "$argument" >> {record}; done
printf '\\n' >> {record}
"""


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
    the repository's own cache keeps the provisioned release in use. Make
    inserts the value into the recipe unquoted, so each path is quoted for the
    recipe's shell: a checkout under a directory whose name contains a space
    would otherwise split into a second command.

    Returns
    -------
    str
        A `UV_ENV` assignment pointing at the repository's caches.
    """
    cache = shlex.quote(str(ROOT / ".uv-cache"))
    tools = shlex.quote(str(ROOT / ".uv-tools"))
    return f"UV_ENV=UV_CACHE_DIR={cache} UV_TOOL_DIR={tools}"


def phrase_scan(tmp_path: Path, status: int = 0) -> str:
    """Return a Make assignment for a phrase-scan stub with *status*.

    The target's third stage runs the shared phrase check. The stub records
    the `check-phrases` argument the real command receives and reports
    *status*, so a test can stand in for a clean scan or a phrase finding.

    Returns
    -------
    str
        A `SPELLING_PHRASE_SCAN` assignment naming the stub.
    """
    stub = recording_stub(tmp_path, "stub-phrase-scan", status)
    return f"SPELLING_PHRASE_SCAN={shlex.quote(str(stub))} check-phrases"


def phrase_index_probe(tmp_path: Path) -> str:
    """Return a Make assignment for a phrase scan that inspects its index.

    The target hands the phrase check a throwaway index. This probe succeeds
    only when one is set and it lists the fixture's untracked document, so the
    target's exit status reports whether the temporary index was wired
    correctly.

    Returns
    -------
    str
        A `SPELLING_PHRASE_SCAN` assignment naming the probe.
    """
    probe = fake_executable(
        tmp_path / "stub-phrase-index-probe",
        "\n".join((
            'test -n "$GIT_INDEX_FILE" || exit 3',
            "git ls-files | grep -qx new-notes.md || exit 4",
            "exit 0",
        ))
        + "\n",
    )
    return f"SPELLING_PHRASE_SCAN={shlex.quote(str(probe))}"


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
