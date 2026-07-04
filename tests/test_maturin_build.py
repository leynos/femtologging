"""Unit tests for maturin pin synchronization and wheel build output."""

from __future__ import annotations

import importlib.metadata as im
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

_EXPECTED_WHEEL_ENTRIES: list[str] = [
    "femtologging-<version>.dist-info/METADATA",
    "femtologging-<version>.dist-info/RECORD",
    "femtologging-<version>.dist-info/WHEEL",
    "femtologging-<version>.dist-info/licenses/LICENSE",
    "femtologging-<version>.dist-info/sboms/<sbom>.cyclonedx.json",
    "femtologging/__init__.py",
    "femtologging/_basic_config.py",
    "femtologging/_compat.py",
    "femtologging/_config_filters.py",
    "femtologging/_femtologging_rs.cpython-<platform>.so",
    "femtologging/_femtologging_rs.pyi",
    "femtologging/_filter_factory.py",
    "femtologging/_log_context.py",
    "femtologging/_rust_compat.py",
    "femtologging/_timed_handler_config.py",
    "femtologging/adapter.py",
    "femtologging/config.py",
    "femtologging/config_protocol.py",
    "femtologging/config_sections.py",
    "femtologging/config_socket.py",
    "femtologging/config_socket_opts.py",
    "femtologging/file_config.py",
    "femtologging/overflow_policy.py",
    "femtologging/unittests/test_overflow_policy.py",
]

_EXPECTED_WHEEL_METADATA: dict[str, object] = {
    "classifiers": [
        "License :: OSI Approved :: ISC License (ISCL)",
        "Operating System :: OS Independent",
        "Programming Language :: Python :: 3",
        "Programming Language :: Rust",
    ],
    "name": "femtologging",
    "requires_dist": [],
    "requires_python": ">=3.12",
    "version": "0.1.0",
}


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

    wheel_path = build_native_wheel_artifact(root, tmp_path / "wheelhouse")
    snapshot_payload = wheel_build_snapshot(wheel_path)
    assert snapshot_payload == {
        "generator": expected,
        "metadata": _EXPECTED_WHEEL_METADATA,
        "wheel": {
            "root_is_purelib": "false",
            "tag": "<platform-tag>",
        },
        "entries": _EXPECTED_WHEEL_ENTRIES,
    }
