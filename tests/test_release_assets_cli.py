"""Command-line contracts for release asset reconciliation."""

from __future__ import annotations

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
