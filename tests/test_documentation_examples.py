"""Verify every example published in femtologging's user-facing documents.

Python examples run, exactly as published, in a fresh interpreter whose
working directory is an empty temporary directory, so file handlers in the
examples never touch the repository. Every other fence is matched to the check
that verifies it; a fence with no check fails the suite rather than going
untested.
"""

from __future__ import annotations

import ast
import re
import tomllib
import typing as typ

import pytest

from tests.documentation_examples import (
    REPOSITORY_ROOT,
    DocumentedExample,
    documented_example,
    load_documented_examples,
    run_example,
)

if typ.TYPE_CHECKING:
    from pathlib import Path

EXAMPLES = load_documented_examples()
PYTHON_EXAMPLES = [example for example in EXAMPLES if example.language == "python"]
OUTPUT_SUFFIX = "-output"
# The crate's lib.rs compiles every Rust fence in these documents as a
# doctest, so a Rust example is verified by `cargo test`.
RUST_DOCTEST_HARNESS = REPOSITORY_ROOT / "rust_extension" / "src" / "lib.rs"
# Mermaid diagrams are rendered and validated by `make nixie`.
NIXIE_LANGUAGES = {"mermaid"}
# Installation commands are published for copying; their checks pin them to
# the release they install instead of running them.
INSTALL_PAIRS = (
    ("readme-install-pip", "guide-install-pip"),
    ("readme-install-uv", "guide-install-uv"),
)
SOURCE_INSTALL = "guide-install-source"
PINNED_INSTALL = "migration-install"


def _has_outcome_assertion(code: str) -> bool:
    """Return whether ``code`` contains an executable ``assert`` statement."""
    return any(isinstance(node, ast.Assert) for node in ast.walk(ast.parse(code)))


def _released_version() -> str:
    """Return the normalized package version recorded in ``uv.lock``."""
    lock = tomllib.loads((REPOSITORY_ROOT / "uv.lock").read_text(encoding="utf-8"))
    return next(
        package["version"]
        for package in lock["package"]
        if package["name"] == "femtologging"
    )


def _verified_ids() -> set[str]:
    """Return the identifiers of every non-Python example with a check."""
    install_ids = {identifier for pair in INSTALL_PAIRS for identifier in pair}
    output_ids = {
        example.id
        for example in EXAMPLES
        if example.language == "text" and example.id.endswith(OUTPUT_SUFFIX)
    }
    other_ids = {
        example.id
        for example in EXAMPLES
        if example.language == "rust" or example.language in NIXIE_LANGUAGES
    }
    return install_ids | output_ids | other_ids | {SOURCE_INSTALL, PINNED_INSTALL}


@pytest.mark.parametrize(
    "example", PYTHON_EXAMPLES, ids=[example.id for example in PYTHON_EXAMPLES]
)
def test_published_python_example_runs(
    example: DocumentedExample, tmp_path: Path
) -> None:
    """Each Python example runs cleanly and checks its own outcome."""
    assert _has_outcome_assertion(example.body), (
        f"{example.source}:{example.line}: {example.id} has no outcome check"
    )
    result = run_example(example, tmp_path)
    assert result.returncode == 0, (
        f"{example.source}:{example.line}: {example.id} failed\n"
        f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
    )


OUTPUT_EXAMPLES = [
    example for example in EXAMPLES if example.id.endswith(OUTPUT_SUFFIX)
]


@pytest.mark.parametrize(
    "output", OUTPUT_EXAMPLES, ids=[example.id for example in OUTPUT_EXAMPLES]
)
def test_published_output_matches_its_example(
    output: DocumentedExample, tmp_path: Path
) -> None:
    """An ``<id>-output`` fence shows exactly what example ``<id>`` prints."""
    assert output.language == "text", f"{output.id} should be a text fence"
    example = documented_example(output.id.removesuffix(OUTPUT_SUFFIX))
    result = run_example(example, tmp_path)
    assert result.stdout + result.stderr == output.body, (
        f"{output.source}:{output.line}: output differs from {example.id}"
    )


def test_every_published_example_has_a_check() -> None:
    """No fence may be published without a check that exercises it."""
    unverified = [
        f"{example.source}:{example.line} {example.id} ({example.language})"
        for example in EXAMPLES
        if example.language != "python" and example.id not in _verified_ids()
    ]
    assert not unverified, f"examples without a check: {unverified}"


def test_rust_examples_compile_as_doctests() -> None:
    """Every document holding a Rust example is compiled by ``cargo test``."""
    harness = RUST_DOCTEST_HARNESS.read_text(encoding="utf-8")
    sources = {example.source for example in EXAMPLES if example.language == "rust"}
    missing = [
        source
        for source in sorted(sources)
        if f'include_str!("../../{source}")' not in harness
    ]
    assert not missing, f"Rust examples are not compiled as doctests: {missing}"


@pytest.mark.parametrize(("readme_id", "guide_id"), INSTALL_PAIRS)
def test_install_commands_request_this_release(readme_id: str, guide_id: str) -> None:
    """The README and guide install the same, current, pre-release."""
    readme = documented_example(readme_id)
    guide = documented_example(guide_id)
    assert readme.body == guide.body, f"{readme_id} and {guide_id} disagree"
    requirement = re.search(r'"femtologging>=(?P<version>[^"]+)"', readme.body)
    assert requirement is not None, f"{readme_id} names no version requirement"
    assert requirement["version"] == _released_version(), (
        f"{readme_id} requests {requirement['version']}, not the release in uv.lock"
    )


def test_source_install_builds_with_maturin() -> None:
    """The documented source install relies on the maturin build backend."""
    example = documented_example(SOURCE_INSTALL)
    pyproject = tomllib.loads(
        (REPOSITORY_ROOT / "pyproject.toml").read_text(encoding="utf-8")
    )
    assert pyproject["build-system"]["build-backend"] == "maturin", (
        "the source install assumes the maturin build backend"
    )
    assert "pip install ." in example.body, "source install must build this checkout"


def test_migration_install_pins_this_release() -> None:
    """The migration guide pins the exact pre-release it describes."""
    example = documented_example(PINNED_INSTALL)
    expected = f'"femtologging=={_released_version()}"'
    assert expected in example.body, f"{PINNED_INSTALL} should pin {expected}"
