"""Contract tests for the development build-tool preflight.

The Concordat ``rust-build-defaults`` rule asks that a host missing the pinned
toolchain or the Linux ``mold`` linker fails with an installation command,
rather than an opaque error deep inside Cargo. These tests hold the Makefile
wiring, run the repository's own check script against stub tools, prove that
``$(CARGO)`` keeps every direct Cargo invocation substitutable, and show that
a refusing preflight stops a build before any build command runs.
"""

from __future__ import annotations

import os
import re
import shlex
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the contract runs fixed make and shell argv
import typing as typ

import pytest

from tests.build_tools_support import ROOT, fake_executable, make_environment
from tests.make_contract_helpers import (
    makefile_report,
    objects,
    prerequisite_map,
    recipe_tokens,
)
from tests.workflow_contract_helpers import depends_on_preflight

if typ.TYPE_CHECKING:
    from pathlib import Path

# Targets that compile Rust through a Cargo command line of their own. The
# lint-lane drivers call Cargo internally, so only their preflight wiring is
# observable from a dry run.
RUST_BUILDING_TARGETS: typ.Final[tuple[str, ...]] = (
    "build",
    "release",
    "test",
    "typecheck",
    "lint-rust-clippy",
    "lint-env-policy",
    "lint-rust-test-docs",
)
CARGO_SUBSTITUTION_TARGETS: typ.Final[tuple[str, ...]] = (
    "release",
    "test",
    "check-fmt",
    "lint-rust-test-docs",
)
# `probe-cargo` must not satisfy the bare-command search, and neither may a
# variable such as `CARGO_LOG`.
BARE_CARGO: typ.Final[re.Pattern[str]] = re.compile(r"(?<![\w-])cargo\b")
PREFLIGHT_COMMAND: typ.Final[re.Pattern[str]] = re.compile(
    r"scripts/check-build-tools\.sh"
)
# The line that carries each target's compile step in its dry run. `build`
# drives `maturin`, and `typecheck`/`test` compile through the `build`
# prerequisite they share, so they observe that same line. The rest expand the
# substituted `$(CARGO)` or drive the lint-lane script.
EXPECTED_COMPILE: typ.Final[dict[str, re.Pattern[str]]] = {
    "build": re.compile(r"\bmaturin\b"),
    "release": re.compile(r"\bprobe-cargo\b"),
    "test": re.compile(r"\bprobe-cargo\b"),
    "typecheck": re.compile(r"\bmaturin\b"),
    "lint-rust-clippy": re.compile(r"\blint_rust_lanes\b"),
    "lint-env-policy": re.compile(r"\blint_rust_lanes\b"),
    "lint-rust-test-docs": re.compile(r"\bprobe-cargo\b"),
}


@pytest.mark.parametrize("target", RUST_BUILDING_TARGETS)
def test_each_rust_building_target_reaches_the_preflight(target: str) -> None:
    """Every target that compiles Rust reaches ``check-build-tools``.

    The same reachability detector proves the mutation case below, so a
    direct-dependency check cannot drift from the mutation's verdict.

    A missing linker otherwise surfaces as an opaque failure deep inside Cargo
    instead of the installation command the preflight prints.
    """
    depends = depends_on_preflight(prerequisite_map())
    assert depends(target) is True, (
        f"{target} must depend on check-build-tools before it compiles"
    )


def test_the_preflight_targets_run_the_repository_scripts() -> None:
    """The Make targets delegate to the checked-in capability scripts."""
    assert recipe_tokens("install-build-tools") == (
        ("@scripts/install-build-tools.sh",),
    ), "install-build-tools must run the installer script"
    assert recipe_tokens("check-build-tools") == (
        ("@scripts/check-build-tools.sh",),
    ), "check-build-tools must run the capability script"


def test_the_preflight_detector_notices_a_removed_dependency(
    tmp_path: Path,
) -> None:
    """The production detector reports a Makefile without the prerequisite.

    Parameters
    ----------
    tmp_path
        Scratch directory that holds the mutated Makefile.
    """
    original = (ROOT / "Makefile").read_text("utf-8")
    assert depends_on_preflight(prerequisite_map())("build") is True, (
        "the repository Makefile must reach the preflight from build"
    )
    mutated = original.replace("build: check-build-tools", "build:", 1)
    assert mutated != original, "the mutation must change the Makefile"
    makefile = tmp_path / "Makefile"
    makefile.write_text(mutated, "utf-8")
    rules = objects(makefile_report(makefile).get("rules"), subject="mutated rules")
    mutated_map = prerequisite_map(rules)
    assert depends_on_preflight(mutated_map)("build") is False, (
        "the detector must notice the removed prerequisite"
    )


def _dry_run(target: str) -> list[str]:
    """Return the commands Make would run for `target` with a probe Cargo.

    Parameters
    ----------
    target
        Makefile target to expand.

    Returns
    -------
    list[str]
        The printed recipe lines, comments included.
    """
    make = shutil.which("make")
    assert make is not None, "make must be installed"
    # ruff: ignore[subprocess-without-shell-equals-true] fixed argv, no shell
    result = subprocess.run(
        [make, "-n", "-B", target, "CARGO=probe-cargo"],
        cwd=ROOT,
        env=make_environment(),
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, f"make -n -B {target} failed:\n{result.stderr}"
    return result.stdout.splitlines()


@pytest.mark.parametrize("target", CARGO_SUBSTITUTION_TARGETS)
def test_each_cargo_invocation_uses_the_injectable_variable(target: str) -> None:
    """A probe ``CARGO`` reaches every direct Cargo command line of `target`.

    A hard-coded ``cargo`` would ignore the substituted executable, so the
    probe could not observe the invocation and the dry run would look green
    while testing nothing.
    """
    lines = _dry_run(target)
    bare = [line for line in lines if BARE_CARGO.search(line)]
    assert not bare, f"{target} invokes a hard-coded cargo: {bare}"
    assert any("probe-cargo" in line for line in lines), (
        f"{target} must expand $(CARGO) on its command lines: {lines}"
    )


def _first_index(lines: list[str], pattern: re.Pattern[str]) -> int | None:
    """Return the index of the first line `pattern` matches, if any.

    Parameters
    ----------
    lines
        The recipe lines Make printed for one target.
    pattern
        The command to look for.

    Returns
    -------
    int | None
        The zero-based index, or ``None`` when no line matches.
    """
    return next((at for at, line in enumerate(lines) if pattern.search(line)), None)


def ordering_fault(lines: list[str], compile_pattern: re.Pattern[str]) -> str | None:
    """Return why a target's dry run fails the preflight ordering, if it does.

    A missing compile line is a fault in its own right: a target that compiles
    through an unexpected command would otherwise pass this check by never
    presenting the line the check looks for.

    Parameters
    ----------
    lines
        The recipe lines Make printed for one target.
    compile_pattern
        The line that carries that target's compile step.

    Returns
    -------
    str | None
        A description of the first fault, or ``None`` when the preflight comes
        first.
    """
    preflight_at = _first_index(lines, PREFLIGHT_COMMAND)
    if preflight_at is None:
        return "the dry run never reaches the preflight"
    compile_at = _first_index(lines, compile_pattern)
    if compile_at is None:
        return "the dry run never reaches the target's compile command"
    if compile_at < preflight_at:
        return "the compile command runs before the preflight"
    return None


@pytest.mark.parametrize("target", RUST_BUILDING_TARGETS)
def test_the_preflight_precedes_the_first_compile(target: str) -> None:
    """A dry run reaches the preflight before that target's compile command."""
    fault = ordering_fault(_dry_run(target), EXPECTED_COMPILE[target])
    assert fault is None, f"{target}: {fault}"


def test_the_ordering_checker_rejects_a_compile_before_the_preflight() -> None:
    """The checker fails a dry run whose compile command runs first.

    The mutation is synthetic so the verdict does not depend on today's recipe
    order, which is the property the check exists to guard.
    """
    compile_pattern = EXPECTED_COMPILE["release"]
    late = ["probe-cargo build --release", "scripts/check-build-tools.sh"]
    assert ordering_fault(late, compile_pattern) == (
        "the compile command runs before the preflight"
    ), late
    early = ["scripts/check-build-tools.sh", "probe-cargo build --release"]
    assert ordering_fault(early, compile_pattern) is None, early


def test_the_ordering_checker_requires_a_compile_command() -> None:
    """A dry run that never compiles is reported rather than passed silently."""
    fault = ordering_fault(
        ["scripts/check-build-tools.sh"], EXPECTED_COMPILE["release"]
    )
    assert fault == "the dry run never reaches the target's compile command", fault


FAKE_UV: typ.Final[str] = """#!/bin/sh
printf '%s\\n' "$*" >> "$UV_LOG"
exit 0
"""

# The `build` recipe runs only `uv` commands, so observing those invocations is
# what distinguishes a correctly gated build from one that reached the recipe.
_REFUSING_PREFLIGHT: typ.Final[str] = "@printf 'preflight refused\\n' >&2; exit 1"


def _mutated_makefile(tmp_path: Path, *, preflight: str) -> Path:
    """Write a copy of the Makefile whose preflight recipe is replaced.

    Parameters
    ----------
    tmp_path
        Scratch directory that holds the mutated Makefile.
    preflight
        Recipe text to put in place of the preflight script invocation.

    Returns
    -------
    Path
        The mutated Makefile.
    """
    original = (ROOT / "Makefile").read_text("utf-8")
    mutated = original.replace("@scripts/check-build-tools.sh", preflight, 1)
    assert mutated != original, (
        "the Makefile must reach the preflight through its recipe"
    )
    makefile = tmp_path / "Makefile"
    makefile.write_text(mutated, "utf-8")
    return makefile


def _stubbed_build_environment(uv: Path, log: Path) -> dict[str, str]:
    """Return an environment whose first ``uv`` on PATH records its arguments.

    Parameters
    ----------
    uv
        The recording stub to put ahead of the inherited PATH.
    log
        File the stub appends each command line to.

    Returns
    -------
    dict[str, str]
        The environment for one fixed Make invocation.
    """
    environment = make_environment(UV_LOG=str(log))
    environment["PATH"] = os.pathsep.join((str(uv.parent), environment["PATH"]))
    return environment


def _recorded_uv_commands(log: Path) -> list[list[str]]:
    """Return the argv of every ``uv`` invocation the stub recorded.

    Parameters
    ----------
    log
        File the recording ``uv`` stub appended its command lines to.

    Returns
    -------
    list[list[str]]
        One token list per invocation, in the order the recipe ran them.
    """
    return [shlex.split(line) for line in log.read_text("utf-8").splitlines()]


def test_a_refused_preflight_stops_the_build_before_uv_runs(
    tmp_path: Path,
) -> None:
    """Make's failure propagation keeps a broken preflight off the build recipe.

    The copy carries one changed recipe: the preflight refuses. The ``build``
    recipe runs ``uv`` and never ``$(CARGO)``, so a recording ``uv`` stub is
    what shows whether the recipe was reached. The control below proves that
    stub observes the recipe when the preflight allows it.
    """
    makefile = _mutated_makefile(tmp_path, preflight=_REFUSING_PREFLIGHT)
    uv = fake_executable(tmp_path / "uv", FAKE_UV)
    log = tmp_path / "uv.log"
    make = shutil.which("make")
    assert make is not None, "make must be installed"
    # ruff: ignore[subprocess-without-shell-equals-true] fixed argv, no shell
    result = subprocess.run(
        [make, "-f", str(makefile), "build"],
        cwd=ROOT,
        env=_stubbed_build_environment(uv, log),
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0, "a refused preflight must fail the build"
    assert "preflight refused" in result.stderr, result.stderr
    assert not log.exists() or not log.read_text("utf-8"), (
        "no build command may run after the preflight refuses"
    )


def _has_adjacent(command: list[str], first: str, second: str) -> bool:
    """Return whether ``first`` immediately precedes ``second`` in ``command``.

    The build recipe inserts optional flags such as ``--locked`` between the
    ``uv`` subcommand and its arguments, so the package command is matched by
    the adjacent pair rather than by a fixed offset.

    Returns
    -------
        bool: ``True`` when the two tokens appear consecutively.
    """
    return any(
        command[index] == first and command[index + 1] == second
        for index in range(len(command) - 1)
    )


def test_a_permitted_preflight_reaches_the_build_recipe(tmp_path: Path) -> None:
    """The ``uv`` stub records the build commands when the preflight passes.

    This control keeps the refusal test honest: its empty log means the recipe
    was gated, not that the stub could never observe a build.
    """
    makefile = _mutated_makefile(tmp_path, preflight="@true")
    uv = fake_executable(tmp_path / "uv", FAKE_UV)
    log = tmp_path / "uv.log"
    make = shutil.which("make")
    assert make is not None, "make must be installed"
    # ruff: ignore[subprocess-without-shell-equals-true] fixed argv, no shell
    result = subprocess.run(
        [make, "-f", str(makefile), "build"],
        cwd=ROOT,
        env=_stubbed_build_environment(uv, log),
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert log.exists(), "the build recipe never invoked the uv stub"
    commands = _recorded_uv_commands(log)
    assert ["venv"] in commands, commands
    # The recipe passes `--locked` between the subcommand and its group flag,
    # so match on the subcommand and the flag rather than on adjacency.
    assert any(
        command[:1] == ["sync"] and "--group" in command for command in commands
    ), commands
    assert any(
        command[:1] == ["run"] and _has_adjacent(command, "maturin", "develop")
        for command in commands
    ), commands
