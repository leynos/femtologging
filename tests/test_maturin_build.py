"""Unit tests for maturin pin synchronization and wheel build output."""

from __future__ import annotations

import importlib.metadata as im
import json
import pathlib as pth
import subprocess  # ruff: ignore[suspicious-subprocess-import] fixed local wheel probe.
import sys
import tomllib
import venv

import pytest

from tests.maturin_compat import (
    build_native_wheel_artifact,
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
    return build_native_wheel_artifact(
        repo_root(), tmp_path_factory.mktemp("production-wheel")
    )


def test_maturin_pins_are_synchronized() -> None:
    """Maturin version pins stay aligned across build entrypoints."""
    pins = read_maturin_pins(repo_root())
    assert len(set(pins.values())) == 1, f"Expected one maturin pin, found {pins!r}"


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


@pytest.mark.timeout(3620)
def test_maturin_wheel_build_snapshot(
    production_wheel: pth.Path,
) -> None:
    """Native wheel metadata and layout match expected maturin output."""
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


@pytest.mark.timeout(3620)
def test_release_wheel_does_not_expose_timed_rotation_hooks(
    production_wheel: pth.Path,
    tmp_path: pth.Path,
) -> None:
    """A fresh environment imports a production wheel without test hooks."""
    environment = tmp_path / "venv"
    venv.EnvBuilder(with_pip=True).create(environment)
    python = environment / (
        "Scripts/python.exe" if sys.platform == "win32" else "bin/python"
    )
    # ruff: ignore[subprocess-without-shell-equals-true] fixed local wheel and interpreter paths.
    subprocess.run(
        [
            str(python),
            "-m",
            "pip",
            "install",
            "--no-deps",
            "--no-index",
            str(production_wheel),
        ],
        check=True,
        capture_output=True,
        text=True,
        timeout=120,
    )
    probe = repo_root() / "scripts/release_wheel_probe.py"
    # ruff: ignore[subprocess-without-shell-equals-true] fixed local probe and interpreter paths.
    subprocess.run(
        [str(python), "-I", str(probe)],
        check=True,
        capture_output=True,
        text=True,
        timeout=30,
        cwd=tmp_path,
    )
