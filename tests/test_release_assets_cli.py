"""Command-line contracts for release asset reconciliation."""

from __future__ import annotations

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
