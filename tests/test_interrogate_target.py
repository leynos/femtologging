"""Behavioural tests for the Makefile's `interrogate` docstring-coverage target.

`make lint` must reach an `interrogate` run over every tracked `.py` file (paths
under interrogate's default exclusions are checked separately), at a threshold
of 100, with a guard that fails on any `MISSED` row (interrogate rounds its
percentage to one decimal, so the threshold alone can pass a lone gap in a large
repository). These tests read the recipe's variables and run the target, so a
dropped invocation or a guard that stops failing breaks a test rather than
silently restoring the gap.
"""

from __future__ import annotations

import os
import re
import shlex
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] - runs git and make with fixed argv
import textwrap
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
MAKEFILE = ROOT / "Makefile"


def _makefile_text() -> str:
    """Return the Makefile source.

    Returns
    -------
    str
        The Makefile's text.
    """
    return MAKEFILE.read_text(encoding="utf-8")


def _variable(name: str) -> str:
    """Return the value a `NAME ?=` or `NAME =` assignment gives, joined.

    Returns
    -------
    str
        The assigned value, with continuation lines joined by single spaces.
    """
    pattern = re.compile(rf"^{name}\s*[:?]?=\s*((?:.*\\\n)*.*)$", re.MULTILINE)
    match = pattern.search(_makefile_text())
    assert match, f"the Makefile must assign {name}"
    return re.sub(r"\\\n\s*", " ", match.group(1)).strip()


def _expanded(name: str) -> str:
    """Return a variable's value with each `$(NAME)` reference expanded.

    Returns
    -------
    str
        The value with every referenced variable substituted, recursively.
    """
    value = _variable(name)
    while match := re.search(r"\$\((\w+)\)", value):
        value = value.replace(match.group(0), _variable(match.group(1)))
    return value


def _prerequisites(target: str) -> list[str]:
    """Return the prerequisites declared for *target*.

    Returns
    -------
    list[str]
        The prerequisite names, in declaration order.
    """
    pattern = re.compile(rf"^{re.escape(target)}:([^=\n]*)$", re.MULTILINE)
    match = pattern.search(_makefile_text())
    assert match, f"the Makefile must declare a {target} target"
    return match.group(1).split("##")[0].split()


def _tracked(*patterns: str) -> list[str]:
    """Return the Git-tracked files matching *patterns*.

    Git is the authority on what the repository owns: a walk would also see
    the virtual environments CI creates beside the sources. Names are read from
    NUL-separated output, so a path containing whitespace stays whole.

    Returns
    -------
    list[str]
        The tracked paths, relative to the repository root.
    """
    output = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] - fixed argv
        ["git", "-C", str(ROOT), "ls-files", "-z", *patterns],  # ruff: ignore[start-process-with-partial-path] - git
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [name for name in output.split("\0") if name]


def test_lint_depends_on_interrogate() -> None:
    """Dropping the call would remove the gate from `make lint` (via `lint-python`)."""
    assert "lint-python" in _prerequisites("lint"), "lint must run lint-python"
    recipe = re.search(
        r"^lint-python:.*\n((?:\t.*\n|\n)*)", _makefile_text(), re.MULTILINE
    )
    assert recipe, "the Makefile must declare a lint-python recipe"
    commands = [line.split() for line in recipe.group(1).splitlines() if line.strip()]
    assert any(
        command[:1] == ["$(MAKE)"] and command[-1] == "interrogate"
        for command in commands
    ), "lint-python must run the interrogate target through $(MAKE)"


def test_interrogate_is_declared_phony() -> None:
    """A file named `interrogate` must not make the target look up to date."""
    declared = re.findall(r"^\.PHONY:((?:.*\\\n)*.*)$", _makefile_text(), re.MULTILINE)
    names = " ".join(declared).replace("\\\n", " ").split()
    assert "interrogate" in names, "interrogate must be a .PHONY target"


def test_every_python_file_is_under_a_target() -> None:
    """A tracked Python file outside `INTERROGATE_TARGETS` is not checked."""
    targets = [Path(t) for t in _expanded("INTERROGATE_TARGETS").split()]
    uncovered = [
        name
        for name in _tracked("*.py")
        if not any(Path(name).is_relative_to(target) for target in targets)
    ]
    assert not uncovered, f"add these to INTERROGATE_TARGETS: {uncovered}"


_MAKE_STATE = frozenset({
    "MAKEFLAGS",
    "MFLAGS",
    "GNUMAKEFLAGS",
    "MAKELEVEL",
    "MAKEOVERRIDES",
    "UV",
    "PYTHON_TARGETS",
})
# Interrogate 1.7.0 skips a path whose first component below a target begins with
# one of these (virtual environments and version-control directories).
_EXCLUDED_PREFIXES = (".tox", ".venv", "venv", ".git", ".hg")


def test_no_tracked_file_hides_behind_a_default_exclusion() -> None:
    """A tracked file interrogate skips by default would never be measured."""
    targets = [Path(t) for t in _expanded("INTERROGATE_TARGETS").split()]
    hidden = [
        name
        for name in _tracked("*.py")
        for target in targets
        if Path(name).is_relative_to(target)
        and Path(name) != target
        and Path(name).relative_to(target).parts[0].startswith(_EXCLUDED_PREFIXES)
    ]
    assert not hidden, f"interrogate skips these by default: {hidden}"


def _assignment(name: str, *words: Path | str) -> str:
    """Build a Make command-line assignment whose value is a shell command.

    Each word is shell-quoted and each dollar sign doubled, so Make hands the
    recipe's shell exactly the words given.

    Returns
    -------
    str
        The ``NAME=value`` argument.
    """
    value = " ".join(shlex.quote(str(word)) for word in words)
    return f"{name}={value.replace('$', '$$')}"


def _make_interrogate(*assignments: str) -> subprocess.CompletedProcess[str]:
    """Run `make interrogate` with the given variable assignments.

    A parent Make's flags and a caller's `UV`, `INTERROGATE*` or `PYTHON_TARGETS`
    overrides would change what the target runs, so those variables are removed
    from the child environment. Any prerequisites the target declares are marked
    up to date with `-o`, so the tests drive the recipe and not another target.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The finished `make` process with its captured output.
    """
    make = shutil.which("make")
    assert make, "make must be on PATH"
    environment = {
        name: value
        for name, value in os.environ.items()
        if name not in _MAKE_STATE and not name.startswith("INTERROGATE")
    }
    skipped = [
        argument
        for prerequisite in _prerequisites("interrogate")
        for argument in ("-o", prerequisite)
    ]
    return subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] - fixed argv
        [make, "-C", str(ROOT), *skipped, "interrogate", *assignments],
        capture_output=True,
        text=True,
        check=False,
        env=environment,
    )


def _run_target(tmp_path: Path, script: str) -> subprocess.CompletedProcess[str]:
    """Run `make interrogate` with the tool replaced by a stub script.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The finished `make` process with its captured output.
    """
    stub = tmp_path / "stub.sh"
    stub.write_text(
        '#!/bin/sh\nprintf "%s\\n" "$@" > "$(dirname "$0")/args.txt"\n'
        + textwrap.dedent(script),
        encoding="utf-8",
    )
    stub.chmod(0o755)
    return _make_interrogate(_assignment("INTERROGATE", stub))


def _run_real_tool(
    tmp_path: Path, definitions: int, undocumented: int
) -> subprocess.CompletedProcess[str]:
    """Run `make interrogate` with the real tool over a generated package.

    The package holds ``definitions`` documented functions and then
    ``undocumented`` functions without a docstring. The tool is the pinned
    release the Makefile runs through ``uv tool run``.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The finished `make` process with its captured output.
    """
    package = tmp_path / "generated"
    package.mkdir()
    documented = "".join(
        f'def documented_{n}() -> None:\n    """Document {n}."""\n\n\n'
        for n in range(definitions)
    )
    bare = "".join(
        f"def bare_{n}() -> None:\n    pass\n\n\n" for n in range(undocumented)
    )
    (package / "mod.py").write_text(
        '"""Generated module."""\n\n\n' + documented + bare, encoding="utf-8"
    )
    return _make_interrogate(_assignment("INTERROGATE_TARGETS", package))


def test_a_documented_package_passes_the_real_gate(tmp_path: Path) -> None:
    """A package whose every definition has a docstring passes the real tool."""
    result = _run_real_tool(tmp_path, definitions=5, undocumented=0)
    assert result.returncode == 0, result.stdout + result.stderr


def test_an_undocumented_function_fails_the_real_gate(tmp_path: Path) -> None:
    """One undocumented function fails the real tool and reports a MISSED row."""
    result = _run_real_tool(tmp_path, definitions=5, undocumented=1)
    assert result.returncode != 0, result.stdout + result.stderr
    assert "MISSED" in result.stdout, result.stdout


def test_a_gap_hidden_by_rounding_still_fails_the_target(tmp_path: Path) -> None:
    """One undocumented function beside 4,000 documented ones still fails.

    The tool alone accepts the report (its percentage rounds to 100.0), so this
    proves the MISSED-row guard is what rejects the lone gap.
    """
    result = _run_real_tool(tmp_path, definitions=4000, undocumented=1)
    assert "RESULT: PASSED" in result.stdout, "the rounded percentage must pass"
    assert result.returncode != 0, "the MISSED guard must still fail the target"
    assert "undocumented definitions remain" in result.stderr, result.stderr


def test_the_target_passes_when_the_tool_reports_no_gap(tmp_path: Path) -> None:
    """A clean report and a zero status pass."""
    result = _run_target(tmp_path, "echo '| f.py | COVERED |'\nexit 0\n")
    assert result.returncode == 0, result.stdout + result.stderr
    assert "| f.py | COVERED |" in result.stdout, "the report must be printed"


def test_the_target_passes_the_configured_flags_and_targets(tmp_path: Path) -> None:
    """The tool receives the configured flags, then the targets.

    The flags are written out because Make has already turned the doubled
    dollar sign of the ignore pattern into one and the shell has removed its
    quotes by the time the tool sees it; `tests/test_lint_tier_contract.py` pins
    the Makefile's own tokens.
    """
    _run_target(tmp_path, "exit 0\n")
    received = (tmp_path / "args.txt").read_text(encoding="utf-8").split()
    expected = [
        "-vv",
        "--fail-under",
        "100",
        "--ignore-regex",
        "^basicConfig$",
        *_expanded("INTERROGATE_TARGETS").split(),
    ]
    assert received == expected, f"the tool received {received}, not {expected}"


def test_the_target_fails_when_the_tool_fails(tmp_path: Path) -> None:
    """A failing tool fails the target."""
    result = _run_target(tmp_path, "echo 'FAILED'\nexit 1\n")
    assert result.returncode != 0, "a failing tool must fail the target"
    assert "FAILED" in result.stdout, "the failing report must still be printed"


@pytest.mark.parametrize(
    "cell",
    ["|  MISSED |", "|   MISSED    |"],
    ids=["interrogate-default-padding", "wider-padding"],
)
def test_the_target_fails_on_a_missed_row_the_percentage_hid(
    tmp_path: Path, cell: str
) -> None:
    """A MISSED row in a report from a successful run still fails the target.

    The guard must not depend on how many spaces pad the status cell.
    """
    result = _run_target(
        tmp_path,
        f"echo '| f.main (L3) {cell}'\necho 'RESULT: PASSED'\nexit 0\n",
    )
    assert result.returncode != 0, "a MISSED row must fail the target"
    assert "undocumented definitions remain" in result.stderr, result.stderr
    assert "f.main (L3)" in result.stdout, "the report must still be printed"
