"""Command-line contracts for release asset reconciliation."""

from __future__ import annotations

import base64
import hashlib
import json
import typing as typ

import pytest

from scripts.release_assets_cli import main

if typ.TYPE_CHECKING:
    from pathlib import Path


@pytest.mark.parametrize(
    "command", ["stage-pypi", "stage-github", "check-fetched", "verify"]
)
def test_pypi_reconciliation_commands_require_an_index(
    tmp_path: Path, capsys: pytest.CaptureFixture[str], command: str
) -> None:
    """PyPI reconciliation cannot silently proceed with an empty index."""
    result = main([
        command,
        "--publish",
        str(tmp_path / "publish"),
        "--assets",
        str(tmp_path / "assets.json"),
    ])

    assert result == 1, "A PyPI reconciliation command requires an index."
    assert capsys.readouterr().err == (
        f"::error title=release-assets::--index is required for {command}\n"
    ), "The missing index is reported as a workflow error."


def test_carryover_can_run_without_a_pypi_index(tmp_path: Path) -> None:
    """Carry-over uses the GitHub snapshot and does not need PyPI data."""
    publish = tmp_path / "publish"
    publish.mkdir()
    assets = tmp_path / "assets.json"
    assets.write_text('{"assets": []}', encoding="utf-8")

    assert (
        main(["carryover", "--publish", str(publish), "--assets", str(assets)]) == 0
    ), "Carry-over can run with only a GitHub asset snapshot."


def _write_index(path: Path, names: tuple[str, ...]) -> None:
    """Write a small PyPI index with valid digests and HTTPS URLs."""
    files = [
        {
            "filename": name,
            "hashes": {"sha256": hashlib.sha256(name.encode()).hexdigest()},
            "url": f"https://files.test/{name}",
        }
        for name in names
    ]
    path.write_text(json.dumps({"files": files}), encoding="utf-8")


def _write_index_digests(path: Path, entries: dict[str, str]) -> None:
    """Write a PyPI index that records the digest of each filename's bytes."""
    files = [
        {
            "filename": name,
            "hashes": {"sha256": digest},
            "url": f"https://files.test/{name}",
        }
        for name, digest in entries.items()
    ]
    path.write_text(json.dumps({"files": files}), encoding="utf-8")


def _write_assets(path: Path, entries: dict[str, str]) -> None:
    """Write the `gh release view --json assets` shape for one release."""
    assets = [
        {"name": name, "digest": f"sha256:{digest}"} for name, digest in entries.items()
    ]
    path.write_text(json.dumps({"assets": assets}), encoding="utf-8")


def _write_bundle(path: Path, subjects: dict[str, str]) -> None:
    """Write a minimal Sigstore bundle attesting the given digests."""
    statement = {
        "subject": [
            {"name": name, "digest": {"sha256": digest}}
            for name, digest in subjects.items()
        ]
    }
    payload = base64.b64encode(json.dumps(statement).encode()).decode()
    path.write_text(
        json.dumps({"dsseEnvelope": {"payload": payload}}), encoding="utf-8"
    )


def _stage_pypi_arguments(tmp_path: Path, *, github_output: Path | None) -> list[str]:
    """Build stage-pypi arguments for a scratch release layout."""
    publish = tmp_path / "publish"
    publish.mkdir()
    (publish / "femtologging.whl").write_bytes(b"wheel")
    (publish / "femtologging.tar.gz").write_bytes(b"sdist")
    _write_index(tmp_path / "index.json", ("femtologging.whl",))
    assets = tmp_path / "assets.json"
    assets.write_text('{"assets": []}', encoding="utf-8")
    github_files = tmp_path / "github-files"
    github_files.mkdir()
    arguments = [
        "stage-pypi",
        "--publish",
        str(publish),
        "--index",
        str(tmp_path / "index.json"),
        "--assets",
        str(assets),
        "--github-files",
        str(github_files),
    ]
    if github_output is not None:
        arguments += ["--github-output", str(github_output)]
    return arguments


def test_stage_pypi_writes_to_the_supplied_github_output(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """The CLI resolves its output file from the argument, not the environment."""
    monkeypatch.delenv("GITHUB_OUTPUT", raising=False)
    destination = tmp_path / "github-output"

    result = main(_stage_pypi_arguments(tmp_path, github_output=destination))

    assert result == 0, "Stage-pypi succeeds without an ambient GITHUB_OUTPUT."
    assert destination.read_text(encoding="utf-8") == "remaining=true\n", (
        "The result is written to the caller-supplied output file."
    )


def test_stage_pypi_requires_an_explicit_github_output(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """An ambient GITHUB_OUTPUT cannot stand in for the explicit argument."""
    ambient = tmp_path / "ambient-output"
    monkeypatch.setenv("GITHUB_OUTPUT", str(ambient))

    result = main(_stage_pypi_arguments(tmp_path, github_output=None))

    assert result == 1, "Stage-pypi refuses to guess its output destination."
    assert capsys.readouterr().err == (
        "::error title=release-assets::--github-output is required for stage-pypi\n"
    ), "The missing output path is reported as a workflow error."
    assert not ambient.exists(), "The ambient output file is ignored."


def test_stage_github_reports_pending_uploads_and_bundle(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """A successful stage-github run reports its pending state and copies bundles."""
    name = "femtologging.whl"
    publish = tmp_path / "publish"
    publish.mkdir()
    (publish / name).write_bytes(b"wheel")
    index = tmp_path / "index.json"
    _write_index(index, (name,))
    assets = tmp_path / "assets.json"
    assets.write_text('{"assets": []}', encoding="utf-8")
    bundles = tmp_path / "bundles"
    bundles.mkdir()
    _write_bundle(
        bundles / "run.sigstore.json",
        {name: hashlib.sha256(name.encode()).hexdigest()},
    )
    github_files = tmp_path / "github-files"
    github_files.mkdir()
    output = tmp_path / "output"
    github_output = tmp_path / "github-output"

    result = main([
        "stage-github",
        "--publish",
        str(publish),
        "--index",
        str(index),
        "--assets",
        str(assets),
        "--bundles",
        str(bundles),
        "--output",
        str(output),
        "--github-files",
        str(github_files),
        "--github-output",
        str(github_output),
    ])

    assert result == 0, "Staging a publishable release succeeds."
    assert capsys.readouterr().out == (f"{name}\thttps://files.test/{name}\n"), (
        "Names already on PyPI are printed as download instructions."
    )
    assert (output / "run.sigstore.json").is_file(), (
        "A bundle that attests the published bytes is staged for upload."
    )
    assert github_output.read_text(encoding="utf-8") == "pending=true\n", (
        "The pending state tells the workflow that assets remain to upload."
    )


def test_check_fetched_accepts_files_matching_the_index(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """A downloaded file that matches its PyPI digest passes the check."""
    name = "femtologging.whl"
    payload = b"downloaded from PyPI"
    digest = hashlib.sha256(payload).hexdigest()
    publish = tmp_path / "publish"
    publish.mkdir()
    (publish / name).write_bytes(b"rebuilt locally")
    index = tmp_path / "index.json"
    _write_index_digests(index, {name: digest})
    assets = tmp_path / "assets.json"
    assets.write_text('{"assets": []}', encoding="utf-8")
    output = tmp_path / "output"
    output.mkdir()
    (output / name).write_bytes(payload)

    result = main([
        "check-fetched",
        "--publish",
        str(publish),
        "--index",
        str(index),
        "--assets",
        str(assets),
        "--output",
        str(output),
    ])

    assert result == 0, "A matching PyPI download passes verification."
    assert capsys.readouterr() == ("", ""), "A successful check prints nothing."


def test_verify_reports_a_fully_reconciled_release(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """Matching digests and attached bundles complete verification."""
    wheel = "femtologging.whl"
    sdist = "femtologging.tar.gz"
    publish = tmp_path / "publish"
    publish.mkdir()
    (publish / wheel).write_bytes(b"wheel bytes")
    (publish / sdist).write_bytes(b"sdist bytes")
    digests = {
        wheel: hashlib.sha256(b"wheel bytes").hexdigest(),
        sdist: hashlib.sha256(b"sdist bytes").hexdigest(),
    }
    index = tmp_path / "index.json"
    _write_index_digests(index, digests)
    assets = tmp_path / "assets.json"
    _write_assets(assets, {**digests, "run.sigstore.json": "0" * 64})
    bundles = tmp_path / "bundles"
    bundles.mkdir()
    _write_bundle(bundles / "run.sigstore.json", digests)

    result = main([
        "verify",
        "--publish",
        str(publish),
        "--index",
        str(index),
        "--assets",
        str(assets),
        "--bundles",
        str(bundles),
    ])

    assert result == 0, "A reconciled, attested release passes verification."
    assert "matching, attested artefact bytes" in capsys.readouterr().out, (
        "A successful verify reports the reconciled destinations."
    )
