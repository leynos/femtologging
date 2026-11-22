"""Unit tests for maturin pin synchronization and wheel build output."""

from __future__ import annotations

import importlib.metadata as im
import json
import pathlib as pth
import subprocess  # ruff: ignore[suspicious-subprocess-import] fixed local wheel probe.
import sys
import tomllib

import pytest

from tests import clean_wheel_env
from tests.maturin_compat import (
    build_native_wheel_artifact,
    install_wheel_into_fresh_venv,
    read_expected_maturin_version,
    read_maturin_pins,
    toolchain_available,
    wheel_build_snapshot,
)

# The expected wheel manifest lives in an external data file so the assertion
# stays readable; ``generator`` carries a placeholder that is substituted with
# the pinned maturin version at assertion time.
_EXPECTED_WHEEL_SNAPSHOT = pth.Path(__file__).with_name("data") / (
    "expected_wheel_snapshot.json"
)
_GENERATOR_PLACEHOLDER = "<maturin-version>"

# The probe imports this module in a child interpreter that pretends the
# standard-library venv and ensurepip modules are absent. It fails if a
# module-level import would break collection before the fixture can skip.
_VENV_BLOCKED_IMPORT_PROBE = """\
import os
import sys


class BlockStdlibVenv:
    def find_spec(self, name, path=None, target=None):
        if name in {"venv", "ensurepip"}:
            raise ModuleNotFoundError(f"No module named {name!r}", name=name)
        return None


sys.meta_path.insert(0, BlockStdlibVenv())
sys.path.insert(0, os.getcwd())
import tests.test_maturin_build
print("imported")
"""


def repo_root() -> pth.Path:
    """Return the repository root path.

    Returns
    -------
    pathlib.Path
        Absolute path to the repository root.

    Examples
    --------
    >>> repo_root().joinpath("pyproject.toml").exists()
    True

    """
    return pth.Path(__file__).resolve().parents[1]


@pytest.fixture(scope="module")
def production_wheel(tmp_path_factory: pytest.TempPathFactory) -> pth.Path:
    """Build the configured production wheel once for packaging checks.

    Parameters
    ----------
    tmp_path_factory : pytest.TempPathFactory
        Pytest factory for creating the isolated wheel output directory.

    Returns
    -------
    pathlib.Path
        Path to the production wheel built for the packaging checks.
    """
    if not toolchain_available():
        pytest.skip("Rust toolchain and maturin are required to build a wheel")
    if sys.version_info >= (3, 15):
        pytest.skip("maturin compatibility snapshot stops at Python 3.14")
    # The clean-environment smoke test needs a stdlib venv, so skip before
    # paying for the release build when the interpreter cannot provide one.
    # Importing ``venv`` at module scope would instead break collection.
    skip_reason = clean_wheel_env.clean_venv_skip_reason()
    if skip_reason is not None:
        pytest.skip(skip_reason)
    return build_native_wheel_artifact(
        repo_root(), tmp_path_factory.mktemp("production-wheel")
    )


def test_clean_venv_skip_reason_names_missing_modules(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """A missing venv or ensurepip produces a skip reason naming both modules."""
    monkeypatch.setattr(clean_wheel_env, "_find_spec", lambda name: None)
    assert clean_wheel_env.clean_venv_available() is False, (
        "simulated absent stdlib modules must make the clean venv unavailable"
    )
    assert clean_wheel_env.clean_venv_skip_reason() == (
        "clean wheel smoke test requires missing stdlib modules: venv, ensurepip"
    ), "the skip reason must name every missing stdlib module"


def test_packaging_tests_import_without_stdlib_venv() -> None:
    """The module imports, and so collects, without venv or ensurepip."""
    # ruff: ignore[subprocess-without-shell-equals-true] fixed probe of the
    # running interpreter against a repository-local module path.
    result = subprocess.run(
        [sys.executable, "-I", "-c", _VENV_BLOCKED_IMPORT_PROBE],
        check=False,
        capture_output=True,
        cwd=repo_root(),
        text=True,
        timeout=60,
    )
    assert result.returncode == 0, (
        "importing the packaging tests must not require the stdlib venv module:\n"
        f"{result.stdout}{result.stderr}"
    )


def test_maturin_pins_are_synchronized() -> None:
    """Maturin pins stay aligned across the manifest, lock, and build entrypoints."""
    pins = read_maturin_pins(repo_root())
    manifest_pins = {
        name: pins[name] for name in ("dev-dependency", "build-system", "uv-lock")
    }
    assert len(set(manifest_pins.values())) == 1, (
        f"maturin versions diverged across {manifest_pins!r}"
    )
    assert pins["build-wheels"] == pins["dev-dependency"], (
        f"the release wheel workflow pins maturin {pins['build-wheels']}, "
        f"not {pins['dev-dependency']}"
    )


def test_persistent_maturin_configuration_is_production_safe() -> None:
    """Production packaging excludes test hooks; coverage opts in explicitly."""
    root = repo_root()
    with (root / "pyproject.toml").open("rb") as project_file:
        project = tomllib.load(project_file)
    assert project["tool"]["maturin"]["features"] == ["python"], (
        "persistent maturin builds must exclude test-util hooks"
    )
    assert project["tool"]["uv"]["package"] is True, (
        "the project must remain an installable uv package"
    )

    coverage_config = tomllib.loads(
        (root / ".github/uv-test-util.toml").read_text(encoding="utf-8")
    )
    assert "package" not in coverage_config, (
        "coverage overrides must not replace project-only uv settings"
    )
    assert coverage_config["config-settings-package"]["femtologging"] == {
        "maturin.build-args": "--features python,test-util"
    }, "coverage builds must opt in to test-util hooks explicitly"


def test_installed_maturin_matches_expected_pin() -> None:
    """The active maturin CLI matches the pinned development dependency."""
    try:
        installed = im.version("maturin")
    except im.PackageNotFoundError:
        pytest.skip()
    expected = read_expected_maturin_version(repo_root())
    assert installed == expected, (
        f"Expected maturin {expected}, but {installed} is installed"
    )


@pytest.mark.timeout(0)
def test_maturin_wheel_build_snapshot(
    production_wheel: pth.Path,
    tmp_path: pth.Path,
) -> None:
    """The wheel matches the recorded layout and imports in a clean environment."""
    root = repo_root()
    expected = read_expected_maturin_version(root)
    if not toolchain_available():
        pytest.skip()
    if sys.version_info >= (3, 15):
        pytest.skip()

    expected_payload = json.loads(_EXPECTED_WHEEL_SNAPSHOT.read_text(encoding="utf-8"))
    assert expected_payload["generator"] == _GENERATOR_PLACEHOLDER, (
        f"{_EXPECTED_WHEEL_SNAPSHOT.name} must keep the generator placeholder so "
        "the pinned maturin version stays the single source of truth"
    )
    expected_payload["generator"] = expected

    snapshot_payload = wheel_build_snapshot(production_wheel)
    assert snapshot_payload == expected_payload, (
        "built wheel metadata and layout must match the recorded manifest in "
        f"{_EXPECTED_WHEEL_SNAPSHOT.name}"
    )
    clean_wheel_env.assert_wheel_imports_in_clean_env(
        production_wheel, tmp_path / "clean-env"
    )


@pytest.mark.timeout(3620)
def test_release_wheel_does_not_expose_timed_rotation_hooks(
    production_wheel: pth.Path,
    tmp_path: pth.Path,
) -> None:
    """A fresh environment imports a production wheel without test hooks."""
    pytest.importorskip(
        "venv", reason="the release wheel probe needs the stdlib venv module"
    )
    pytest.importorskip(
        "ensurepip", reason="the release wheel probe needs pip in the new environment"
    )
    python = install_wheel_into_fresh_venv(tmp_path / "venv", production_wheel)
    probe = repo_root() / "scripts/release_wheel_probe.py"
    # ruff: ignore[subprocess-without-shell-equals-true] fixed local probe and interpreter paths.
    probe_result = subprocess.run(
        [str(python), "-I", str(probe)],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
        cwd=tmp_path,
    )
    assert probe_result.returncode == 0, (
        f"release wheel probe failed:\n{probe_result.stdout}{probe_result.stderr}"
    )
