"""Unit contracts for the release asset filesystem readers."""

from __future__ import annotations

import hashlib
import json
import typing as typ

import pytest

from scripts.release_asset_errors import (
    MalformedDigestError,
    MissingDigestError,
    UnsafeArtifactNamesError,
)
from scripts.release_asset_io import INDEX_URL, local_names, read_pypi_index

if typ.TYPE_CHECKING:
    from pathlib import Path


def _digest(name: str) -> str:
    """Return a deterministic valid SHA-256 digest for a fixture filename."""
    return hashlib.sha256(name.encode()).hexdigest()


def test_local_names_returns_only_sorted_regular_files(tmp_path: Path) -> None:
    """Local staging ignores directories and returns names in sorted order."""
    (tmp_path / "b.whl").write_bytes(b"b")
    (tmp_path / "a.whl").write_bytes(b"a")
    (tmp_path / "nested").mkdir()

    assert local_names(tmp_path) == ["a.whl", "b.whl"], (
        "Local staging ignores directories and returns names in sorted order."
    )


@pytest.mark.parametrize(
    "name",
    ["-leading-dash.whl", "space name.whl", "wheel;touch pwned.whl"],
)
def test_local_names_rejects_unsafe_filenames(tmp_path: Path, name: str) -> None:
    """Names that could confuse a shell step are refused.

    A directory entry name cannot contain a path separator, so traversal is
    excluded by construction rather than by this check.
    """
    (tmp_path / name).write_bytes(b"unsafe")

    with pytest.raises(UnsafeArtifactNamesError, match="refusing unsafe artefact"):
        local_names(tmp_path)


def test_read_pypi_index_resolves_relative_urls_against_the_simple_index(
    tmp_path: Path,
) -> None:
    """A relative PEP 691 file URL resolves against the Simple index URL."""
    name = "femtologging-1.0.0-py3-none-any.whl"
    index = tmp_path / "index.json"
    index.write_text(
        json.dumps({
            "files": [
                {
                    "filename": name,
                    "hashes": {"sha256": _digest(name)},
                    "url": name,
                }
            ]
        }),
        encoding="utf-8",
    )

    record = read_pypi_index(index)[name]

    assert record.url == f"{INDEX_URL}{name}", (
        "A relative PEP 691 file URL resolves against the Simple index URL."
    )


def test_read_pypi_index_rejects_a_missing_digest(tmp_path: Path) -> None:
    """A PyPI file entry without a SHA-256 digest cannot be reconciled."""
    name = "femtologging-1.0.0-py3-none-any.whl"
    index = tmp_path / "index.json"
    index.write_text(
        json.dumps({
            "files": [{"filename": name, "hashes": {}, "url": "https://f.test/w"}]
        }),
        encoding="utf-8",
    )

    with pytest.raises(MissingDigestError, match="does not contain a SHA-256"):
        read_pypi_index(index)


def test_read_pypi_index_reports_a_malformed_digest(tmp_path: Path) -> None:
    """A non-hexadecimal digest is rejected instead of being compared."""
    name = "femtologging-1.0.0-py3-none-any.whl"
    index = tmp_path / "index.json"
    index.write_text(
        json.dumps({
            "files": [
                {
                    "filename": name,
                    "hashes": {"sha256": "not-a-digest"},
                    "url": "https://f.test/wheel",
                }
            ]
        }),
        encoding="utf-8",
    )

    with pytest.raises(MalformedDigestError, match="malformed SHA-256"):
        read_pypi_index(index)
