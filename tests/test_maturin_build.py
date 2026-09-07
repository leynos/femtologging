"""Unit tests for maturin pin synchronization and wheel build output."""

from __future__ import annotations

import importlib.metadata as im
import json
import pathlib as pth
import sys

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


def test_maturin_pins_are_synchronized() -> None:
    """Maturin version pins stay aligned across build entrypoints."""
    pins = read_maturin_pins(repo_root())
    assert len(set(pins.values())) == 1, f"Expected one maturin pin, found {pins!r}"


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
    tmp_path: pth.Path,
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

    wheel_path = build_native_wheel_artifact(root, tmp_path / "wheelhouse")
    snapshot_payload = wheel_build_snapshot(wheel_path)
    assert snapshot_payload == expected_payload, (
        "built wheel metadata and layout must match the recorded manifest in "
        f"{_EXPECTED_WHEEL_SNAPSHOT.name}"
    )
