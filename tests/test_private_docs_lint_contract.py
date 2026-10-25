"""Exercise the private Rust documentation policy through the real lint gate."""

from __future__ import annotations

import os
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the contract runs the real target
import tempfile
import tomllib
import typing as typ
from pathlib import Path

import pytest

_REPO_ROOT: typ.Final = Path(__file__).resolve().parent.parent
_PRODUCTION_MANIFEST: typ.Final = _REPO_ROOT / "rust_extension" / "Cargo.toml"
_LINT_NAME: typ.Final = "missing_docs_in_private_items"
_FEATURES: typ.Final = (
    "extension-module",
    "python",
    "log-compat",
    "tracing-compat",
    "test-util",
)
_LINT_GATE_TIMEOUT_SECONDS: typ.Final = 600
# This contract runs the lint gate twice, once for each fixture variant.
# Allow both subprocess budgets to expire plus time for test setup and teardown.
_CONTRACT_TIMEOUT_SECONDS: typ.Final = 2 * _LINT_GATE_TIMEOUT_SECONDS + 20


def _private_docs_level() -> str:
    """Return the private-doc lint level declared by the production crate."""
    manifest = tomllib.loads(_PRODUCTION_MANIFEST.read_text(encoding="utf-8"))
    lint_level = manifest["lints"]["clippy"].get("missing_docs_in_private_items")
    assert lint_level == "deny", (
        f"{_PRODUCTION_MANIFEST} must deny missing_docs_in_private_items, "
        f"found {lint_level!r}"
    )
    return lint_level


def _write_fixture(directory: Path, lint_level: str, source: str) -> Path:
    """Create a dependency-free crate with the production lint policy."""
    source_directory = directory / "src"
    source_directory.mkdir(parents=True)
    (source_directory / "lib.rs").write_text(source, encoding="utf-8")
    features = "\n".join(f"{feature} = []" for feature in _FEATURES)
    manifest = directory / "Cargo.toml"
    manifest.write_text(
        "\n".join([
            "[package]",
            'name = "private_docs_fixture"',
            'version = "0.1.0"',
            'edition = "2024"',
            "",
            "[lints.clippy]",
            f'missing_docs_in_private_items = "{lint_level}"',
            "",
            "[features]",
            "default = []",
            features,
            "",
        ]),
        encoding="utf-8",
    )
    return manifest


def _run_lint_gate(manifest: Path) -> subprocess.CompletedProcess[str]:
    """Run the no-feature Clippy lane against a fixture manifest."""
    make_executable = shutil.which("make")
    if make_executable is None:
        message = "make unavailable"
        raise RuntimeError(message)

    environment = dict(os.environ)
    environment["CARGO_TARGET_DIR"] = str(
        _REPO_ROOT / "target" / "private-docs-policy-clippy"
    )
    return subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed Make target and manifest arguments
        [
            make_executable,
            "lint-rust-clippy",
            f"RUST_MANIFEST={manifest}",
            "RUST_LINT_FEATURE_LANES=none",
        ],
        cwd=_REPO_ROOT,
        env=environment,
        capture_output=True,
        check=False,
        timeout=_LINT_GATE_TIMEOUT_SECONDS,
        text=True,
    )


@pytest.mark.timeout(_CONTRACT_TIMEOUT_SECONDS)
def test_private_docs_deny_is_exercised_by_make_clippy_lanes() -> None:
    """The lint rejects missing docs and accepts docs through Make's lane driver."""
    lint_level = _private_docs_level()
    fixture_root = _REPO_ROOT / "target" / "private-docs-policy-fixtures"
    fixture_root.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(dir=fixture_root) as temporary_directory:
        directory = Path(temporary_directory)
        manifest = _write_fixture(
            directory,
            lint_level,
            "#[allow(dead_code)]\nstruct Missing;\n",
        )
        source_file = manifest.parent / "src" / "lib.rs"
        undocumented = _run_lint_gate(manifest)
        undocumented_output = undocumented.stdout + undocumented.stderr
        assert undocumented.returncode != 0, (
            "the private-doc policy accepted an undocumented private item"
        )
        assert _LINT_NAME in undocumented_output, (
            "Clippy failed for a reason other than the private-doc policy:\n"
            f"{undocumented_output}"
        )
        assert "Rust lint lane failed: none" in undocumented_output, (
            "the failing private-doc fixture did not pass through the lane driver:\n"
            f"{undocumented_output}"
        )

        source_file.write_text(
            "/// A private type used to verify the documentation policy.\n"
            "#[allow(dead_code)]\n"
            "struct Present;\n",
            encoding="utf-8",
        )
        documented = _run_lint_gate(manifest)
        assert documented.returncode == 0, (
            "Clippy rejected the documented fixture:\n"
            f"{documented.stdout}{documented.stderr}"
        )
