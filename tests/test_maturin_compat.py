"""Unit tests for the maturin compatibility helpers.

``read_maturin_pins`` compares the development pin, the build-system bound and
the release workflow's literal against the version recorded in ``uv.lock``. The
lock lookup is the part that reads a file format rather than a regex, so these
tests drive it with temporary lock files: a missing package, a duplicated
package, a non-string version, and a valid record. None of them depend on the
repository's own ``uv.lock``.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.maturin_compat import _locked_maturin_version

if typ.TYPE_CHECKING:
    from pathlib import Path


def _lock(*packages: str) -> str:
    """Return a minimal lock file body containing `packages` verbatim."""
    return "version = 1\n" + "".join(
        f"[[package]]\n{package}\n" for package in packages
    )


def test_a_lock_without_maturin_is_rejected(tmp_path: Path) -> None:
    """A lock that records no maturin package cannot satisfy the pin check."""
    lockfile = tmp_path / "uv.lock"
    lockfile.write_text(_lock('name = "pytest"\nversion = "9.1.1"'), encoding="utf-8")

    with pytest.raises(AssertionError, match="Expected one maturin package"):
        _locked_maturin_version(lockfile)


def test_a_lock_with_two_maturin_packages_is_rejected(tmp_path: Path) -> None:
    """Two maturin records are ambiguous, so the helper refuses to guess."""
    lockfile = tmp_path / "uv.lock"
    lockfile.write_text(
        _lock(
            'name = "maturin"\nversion = "1.13.3"',
            'name = "maturin"\nversion = "1.14.0"',
        ),
        encoding="utf-8",
    )

    with pytest.raises(AssertionError, match="found 2"):
        _locked_maturin_version(lockfile)


def test_a_non_string_locked_version_is_a_type_error(tmp_path: Path) -> None:
    """A malformed record is reported as a type problem, not a missing pin."""
    lockfile = tmp_path / "uv.lock"
    lockfile.write_text(_lock('name = "maturin"\nversion = 113'), encoding="utf-8")

    with pytest.raises(TypeError, match="carries no string version"):
        _locked_maturin_version(lockfile)


def test_a_valid_lock_returns_the_recorded_version(tmp_path: Path) -> None:
    """The helper returns the single recorded maturin version."""
    lockfile = tmp_path / "uv.lock"
    lockfile.write_text(
        _lock(
            'name = "pytest"\nversion = "9.1.1"',
            'name = "maturin"\nversion = "1.13.3"',
        ),
        encoding="utf-8",
    )

    assert _locked_maturin_version(lockfile) == "1.13.3", (
        "the helper must return the single recorded maturin version"
    )
