"""Contract tests for the development build-tool preflight.

The Concordat ``rust-build-defaults`` rule asks that a host missing the pinned
toolchain or the Linux ``mold`` linker fails with an installation command,
rather than an opaque error deep inside Cargo. These tests hold the Makefile
wiring, run the repository's own check script against stub tools, prove that
``$(CARGO)`` keeps every direct Cargo invocation substitutable, and show that
a refusing preflight stops a build before any compiler runs.
"""

from __future__ import annotations

import re
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
    sole_recipe_rule,
    text_sequence,
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
COMPILE_COMMAND: typ.Final[re.Pattern[str]] = re.compile(
    r"\b(maturin|lint_rust_lanes)\b|(?<![\w-])probe-cargo\b"
)


def _prerequisites(target: str) -> tuple[str, ...]:
    """Return the parsed prerequisites of a Makefile target.

    Parameters
    ----------
    target
        Name of the Makefile target.

    Returns
    -------
    tuple[str, ...]
        The prerequisite names in declaration order.
    """
    rule = sole_recipe_rule(target, require_recipes=False)
    return text_sequence(rule.get("prerequisites"), subject=f"{target} prerequisites")


@pytest.mark.parametrize("target", RUST_BUILDING_TARGETS)
def test_each_rust_building_target_reaches_the_preflight(target: str) -> None:
    """Every target that compiles Rust depends on ``check-build-tools``.

    A missing linker otherwise surfaces as an opaque failure deep inside Cargo
    instead of the installation command the preflight prints.
    """
    assert "check-build-tools" in _prerequisites(target), (
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
    assert depends_on_preflight()("build") is True, (
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


@pytest.mark.parametrize("target", RUST_BUILDING_TARGETS)
def test_the_preflight_precedes_the_first_compile(target: str) -> None:
    """A dry run reaches the preflight before any compiler invocation."""
    lines = _dry_run(target)
    preflight_at = next(
        (at for at, line in enumerate(lines) if "scripts/check-build-tools.sh" in line),
        None,
    )
    assert preflight_at is not None, f"{target} never reaches the preflight: {lines}"
    compile_at = next(
        (at for at, line in enumerate(lines) if COMPILE_COMMAND.search(line)),
        None,
    )
    if compile_at is not None:
        assert preflight_at < compile_at, (
            f"{target} must check the build tools before compiling: {lines}"
        )


FAKE_CARGO: typ.Final[str] = """#!/bin/sh
printf '%s\\n' "$*" >> "$CARGO_LOG"
exit 0
"""


def test_a_refused_preflight_stops_the_build_before_cargo_runs(
    tmp_path: Path,
) -> None:
    """Make's failure propagation keeps a broken preflight off the compiler.

    The copy carries one changed recipe: the preflight refuses. If the
    prerequisite ordering were wrong, the fake Cargo would still record a
    build invocation.
    """
    original = (ROOT / "Makefile").read_text("utf-8")
    mutated = original.replace(
        "@scripts/check-build-tools.sh",
        "@printf 'preflight refused\\n' >&2; exit 1",
        1,
    )
    assert mutated != original, (
        "the Makefile must reach the preflight through its recipe"
    )
    makefile = tmp_path / "Makefile"
    makefile.write_text(mutated, "utf-8")
    fake = fake_executable(tmp_path / "probe-cargo", FAKE_CARGO)
    log = tmp_path / "cargo.log"
    make = shutil.which("make")
    assert make is not None, "make must be installed"
    # ruff: ignore[subprocess-without-shell-equals-true] fixed argv, no shell
    result = subprocess.run(
        [make, "-f", str(makefile), "build", f"CARGO={fake}"],
        cwd=ROOT,
        env=make_environment(CARGO_LOG=str(log)),
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0, "a refused preflight must fail the build"
    assert "preflight refused" in result.stderr, result.stderr
    assert not log.exists() or not log.read_text("utf-8"), (
        "no compiler may run after the preflight refuses"
    )
