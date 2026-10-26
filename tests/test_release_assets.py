"""Unit contracts for canonical-byte release reconciliation."""

from __future__ import annotations

import hashlib
import json
import typing as typ

import pytest

from scripts.release_assets import (
    ReleaseAssetError,
    Source,
    State,
    carryover,
    check_fetched,
    plan,
    read_github_assets,
    read_pypi_index,
    stage_github,
    stage_pypi,
    verify,
)

if typ.TYPE_CHECKING:
    from pathlib import Path


def _digest(contents: bytes) -> str:
    """Return a content digest for compact temporary fixtures."""
    return hashlib.sha256(contents).hexdigest()


def _write_index(path: Path, entries: dict[str, tuple[bytes, str]]) -> None:
    """Write a minimal PEP 691 index from filenames, bytes, and URLs."""
    files = [
        {"filename": name, "hashes": {"sha256": _digest(contents)}, "url": url}
        for name, (contents, url) in entries.items()
    ]
    path.write_text(json.dumps({"files": files}), encoding="utf-8")


def _write_assets(path: Path, entries: dict[str, str | None]) -> None:
    """Write the `gh release view --json assets` shape used by the helpers."""
    assets = [
        {"name": name, "digest": f"sha256:{digest}" if digest else None}
        for name, digest in entries.items()
    ]
    path.write_text(json.dumps({"assets": assets}), encoding="utf-8")


def _write(path: Path, contents: bytes) -> str:
    """Write bytes to a fixture and return their SHA-256 digest."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(contents)
    return _digest(contents)


def test_plan_prefers_pypi_then_github_then_this_build() -> None:
    """A filename's canonical bytes come from the earliest existing target."""
    result = plan(
        ["both.whl", "github.tar.gz", "local.whl"],
        {"both.whl"},
        {"both.whl", "github.tar.gz"},
    )
    assert result.canonical == {
        "both.whl": Source.PYPI,
        "github.tar.gz": Source.GITHUB,
        "local.whl": Source.LOCAL,
    }, "A filename's canonical bytes come from the earliest existing target."
    assert result.pypi_uploads == {"github.tar.gz", "local.whl"}, (
        "A filename's canonical bytes come from the earliest existing target."
    )
    assert result.github_uploads == {"local.whl"}, (
        "A filename's canonical bytes come from the earliest existing target."
    )


def test_stage_pypi_skips_published_names_and_carries_github_bytes(
    tmp_path: Path,
) -> None:
    """PyPI names are removed and an existing GitHub asset replaces a rebuild."""
    publish = tmp_path / "publish"
    published = publish / "femtologging.whl"
    carry = publish / "femtologging.tar.gz"
    _write(published, b"rebuilt wheel")
    _write(carry, b"rebuilt sdist")
    github_files = tmp_path / "github-files"
    github_digest = _write(github_files / carry.name, b"canonical sdist")
    index = tmp_path / "index.json"
    _write_index(
        index, {published.name: (b"published wheel", "https://files.test/wheel")}
    )
    assets = tmp_path / "assets.json"
    _write_assets(assets, {carry.name: github_digest})
    state = State.load(publish, index, assets)

    assert carryover(state) == [carry.name], (
        "PyPI names are removed and an existing GitHub asset replaces a rebuild."
    )
    assert stage_pypi(state, github_files) is True, (
        "PyPI names are removed and an existing GitHub asset replaces a rebuild."
    )
    assert not published.exists(), "PyPI already owns the wheel filename"
    assert carry.read_bytes() == b"canonical sdist", (
        "PyPI names are removed and an existing GitHub asset replaces a rebuild."
    )


def test_stage_pypi_succeeds_when_every_filename_is_already_published(
    tmp_path: Path,
) -> None:
    """An empty PyPI upload set is a successful idempotent result."""
    publish = tmp_path / "publish"
    name = "femtologging.whl"
    _write(publish / name, b"rebuilt")
    index = tmp_path / "index.json"
    _write_index(index, {name: (b"published", "https://files.test/wheel")})
    assets = tmp_path / "assets.json"
    _write_assets(assets, {})
    github_files = tmp_path / "github-files"
    github_files.mkdir()

    assert stage_pypi(State.load(publish, index, assets), github_files) is False, (
        "An empty PyPI upload set is a successful idempotent result."
    )
    assert not list(publish.iterdir()), (
        "An empty PyPI upload set is a successful idempotent result."
    )


def test_stage_github_uses_pypi_bytes_and_stages_only_absent_bundles(
    tmp_path: Path,
) -> None:
    """GitHub gets canonical PyPI bytes and only new provenance bundles."""
    publish = tmp_path / "publish"
    local_wheel = "femtologging-local.whl"
    pypi_sdist = "femtologging.tar.gz"
    _write(publish / local_wheel, b"new local wheel")
    _write(publish / pypi_sdist, b"rebuilt sdist")
    index = tmp_path / "index.json"
    _write_index(
        index,
        {pypi_sdist: (b"published canonical sdist", "https://files.test/sdist")},
    )
    assets = tmp_path / "assets.json"
    old_bundle = "femtologging-old.sigstore.json"
    _write_assets(assets, {old_bundle: _digest(b"old")})
    bundles = tmp_path / "bundles"
    _write(bundles / old_bundle, b"rebuilt old bundle")
    new_bundle = "femtologging-run-12.sigstore.json"
    _write(bundles / new_bundle, b"new bundle")
    output = tmp_path / "github-upload"

    fetch = stage_github(State.load(publish, index, assets), bundles, output)

    assert fetch == [(pypi_sdist, "https://files.test/sdist")], (
        "GitHub gets canonical PyPI bytes and only new provenance bundles."
    )
    assert (output / local_wheel).read_bytes() == b"new local wheel", (
        "GitHub gets canonical PyPI bytes and only new provenance bundles."
    )
    assert not (output / old_bundle).exists(), "existing bundle names are left alone"
    assert (output / new_bundle).read_bytes() == b"new bundle", (
        "GitHub gets canonical PyPI bytes and only new provenance bundles."
    )


def test_stage_github_rejects_non_https_pypi_urls(tmp_path: Path) -> None:
    """A file with an insecure download URL is never staged for GitHub."""
    publish = tmp_path / "publish"
    name = "femtologging.whl"
    _write(publish / name, b"local bytes")
    index = tmp_path / "index.json"
    _write_index(index, {name: (b"published", "http://files.test/wheel")})
    assets = tmp_path / "assets.json"
    _write_assets(assets, {})
    bundles = tmp_path / "bundles"
    bundles.mkdir()

    with pytest.raises(ReleaseAssetError, match="every PyPI download must use HTTPS"):
        stage_github(State.load(publish, index, assets), bundles, tmp_path / "out")


def test_malformed_digests_are_rejected(tmp_path: Path) -> None:
    """Only valid SHA-256 digests are accepted from both destinations."""
    assets = tmp_path / "assets.json"
    _write_assets(
        assets,
        {
            "old-algorithm.whl": None,
            "malformed.whl": None,
        },
    )
    document = json.loads(assets.read_text(encoding="utf-8"))
    document["assets"][0]["digest"] = f"sha1:{_digest(b'one')}"
    document["assets"][1]["digest"] = "sha256:not-a-digest"
    assets.write_text(json.dumps(document), encoding="utf-8")
    assert read_github_assets(assets) == {
        "old-algorithm.whl": None,
        "malformed.whl": None,
    }, "Only valid SHA-256 digests are accepted from both destinations."

    index = tmp_path / "index.json"
    index.write_text(
        json.dumps({
            "files": [
                {
                    "filename": "bad.whl",
                    "hashes": {"sha256": "short"},
                    "url": "https://files.test/bad",
                }
            ]
        }),
        encoding="utf-8",
    )
    with pytest.raises(ReleaseAssetError, match="malformed SHA-256"):
        read_pypi_index(index)


def test_destination_hashes_are_normalized_before_comparison(tmp_path: Path) -> None:
    """Uppercase hexadecimal digests compare in their canonical lowercase form."""
    content_digest = _digest(b"canonical bytes")
    index = tmp_path / "index.json"
    index.write_text(
        json.dumps({
            "files": [
                {
                    "filename": "femtologging.whl",
                    "hashes": {"sha256": content_digest.upper()},
                    "url": "https://files.test/wheel",
                }
            ]
        }),
        encoding="utf-8",
    )
    assets = tmp_path / "assets.json"
    _write_assets(assets, {"femtologging.whl": content_digest.upper()})

    assert read_pypi_index(index)["femtologging.whl"].sha256 == content_digest, (
        "Uppercase hexadecimal digests compare in their canonical lowercase form."
    )
    assert read_github_assets(assets)["femtologging.whl"] == content_digest, (
        "Uppercase hexadecimal digests compare in their canonical lowercase form."
    )


def test_check_fetched_and_verify_detect_digest_mismatches(tmp_path: Path) -> None:
    """Fetched files and both published destinations must have matching hashes."""
    publish = tmp_path / "publish"
    name = "femtologging.whl"
    digest = _write(publish / name, b"same package bytes")
    index = tmp_path / "index.json"
    _write_index(index, {name: (b"same package bytes", "https://files.test/wheel")})
    assets = tmp_path / "assets.json"
    _write_assets(assets, {})
    state = State.load(publish, index, assets)
    output = tmp_path / "github-upload"
    _write(output / name, b"same package bytes")
    check_fetched(state, output)

    (output / name).write_bytes(b"different bytes")
    with pytest.raises(ReleaseAssetError, match="has sha256"):
        check_fetched(state, output)

    _write_assets(assets, {name: "0" * 64})
    bundles = tmp_path / "bundles"
    bundles.mkdir()
    problems = verify(State.load(publish, index, assets), bundles)
    assert any("differs" in issue and name in issue for issue in problems), (
        "Fetched files and both published destinations must have matching hashes."
    )
    assert digest == _digest(b"same package bytes"), (
        "Fetched files and both published destinations must have matching hashes."
    )


def test_stage_github_blocks_pypi_and_github_digest_disagreement(
    tmp_path: Path,
) -> None:
    """An existing GitHub asset that differs from PyPI blocks release staging."""
    publish = tmp_path / "publish"
    name = "femtologging.whl"
    _write(publish / name, b"fresh local rebuild")
    github_files = tmp_path / "github-files"
    github_digest = _write(github_files / name, b"old GitHub bytes")
    index = tmp_path / "index.json"
    _write_index(index, {name: (b"canonical PyPI bytes", "https://files.test/wheel")})
    assets = tmp_path / "assets.json"
    _write_assets(assets, {name: github_digest})
    bundles = tmp_path / "bundles"
    bundles.mkdir()

    with pytest.raises(ReleaseAssetError, match="differs between PyPI and the GitHub"):
        stage_github(
            State.load(publish, index, assets),
            bundles,
            tmp_path / "github-upload",
            github_files,
        )
