"""Exercise draft, carry-over, and publication steps with a fake `gh`."""

from __future__ import annotations

import hashlib
import json
import os
import sys
import typing as typ

import pytest

from tests.release_workflow_helpers import (
    run_bash,
    step_script,
    workflow_command_environment,
    write_argument_recorder,
)

if typ.TYPE_CHECKING:
    import subprocess
    from pathlib import Path

RELEASE_WORKFLOW = ".github/workflows/release.yml"
_TAG = "v0.2.0-beta1"
_DRAFT_STEP = (RELEASE_WORKFLOW, "draft-release", "Create or reuse the tag's release")
_SNAPSHOT_STEP = (
    RELEASE_WORKFLOW,
    "draft-release",
    "Snapshot GitHub assets and carry over existing bytes",
)
_STAGE_STEP = (RELEASE_WORKFLOW, "publish-release", "Stage missing GitHub assets")
_UPLOAD_STEP = (
    RELEASE_WORKFLOW,
    "publish-release",
    "Upload missing assets to the GitHub Release",
)
_PUBLISH_STEP = (RELEASE_WORKFLOW, "publish-release", "Publish the GitHub Release")

pytestmark = pytest.mark.skipif(
    sys.platform == "win32",
    reason="The release workflow's shell steps run under Bash on Linux.",
)


def _install_fake_gh(directory: Path) -> Path:
    """Install an argument-recording gh shim with scripted release reads."""
    body = (
        "import json, os, pathlib, shutil, sys\n"
        "args = sys.argv[1:]\n"
        "with pathlib.Path(os.environ['GH_CALLS']).open(\n"
        "    'a', encoding='utf-8'\n"
        ") as out:\n"
        "    out.write(json.dumps(args) + '\\n')\n"
        "if args[:1] == ['api']:\n"
        "    status = int(os.environ.get('GH_RELEASE_LIST_STATUS', '0'))\n"
        "    if status:\n"
        "        raise SystemExit(status)\n"
        "    state = os.environ.get('RELEASE_STATE', 'missing')\n"
        "    count = int(os.environ.get('RELEASE_COUNT', '1'))\n"
        "    releases = (\n"
        "        [{'tag_name': os.environ['TAG_NAME'],\n"
        "          'draft': state == 'draft'}] * count\n"
        "        if state != 'missing' else []\n"
        "    )\n"
        "    print(json.dumps([releases]))\n"
        "    raise SystemExit(0)\n"
        "if args[:2] == ['release', 'view']:\n"
        "    if '--json' in args:\n"
        "        print(\n"
        "            pathlib.Path(os.environ['GH_ASSETS_JSON'])\n"
        "            .read_text(encoding='utf-8')\n"
        "        )\n"
        "        raise SystemExit(0)\n"
        "    raise SystemExit(int(os.environ.get('GH_VIEW_STATUS', '0')))\n"
        "if args[:2] == ['release', 'download']:\n"
        "    name = args[args.index('--pattern') + 1]\n"
        "    target = pathlib.Path(args[args.index('--dir') + 1]) / name\n"
        "    source = pathlib.Path(os.environ['GH_ASSET_FILES']) / name\n"
        "    target.parent.mkdir(parents=True, exist_ok=True)\n"
        "    shutil.copyfile(source, target)\n"
    )
    return write_argument_recorder(directory, "gh", body)


def _run_gh_step(
    tmp_path: Path,
    step: tuple[str, str, str],
    *,
    overrides: dict[str, str] | None = None,
) -> tuple[subprocess.CompletedProcess[str], list[list[str]]]:
    """Run one workflow step with fake gh and return its recorded arguments."""
    tools = tmp_path / "fake-tools"
    _install_fake_gh(tools)
    calls = tmp_path / "gh-calls.jsonl"
    assets_json = tmp_path / "fake-assets.json"
    if not assets_json.exists():
        assets_json.write_text('{"assets": []}', encoding="utf-8")
    asset_files = tmp_path / "fake-assets"
    asset_files.mkdir(exist_ok=True)
    environment = workflow_command_environment(
        step,
        tmp_path,
        {
            "PATH": os.pathsep.join((str(tools), os.environ.get("PATH", os.defpath))),
            "GH_CALLS": str(calls),
            "GH_ASSETS_JSON": str(assets_json),
            "GH_ASSET_FILES": str(asset_files),
            "RELEASE_REPOSITORY": "leynos/femtologging",
            "RELEASE_TEMP_PATH": str(tmp_path),
            "GH_TOKEN": "test-token",
            "TAG_NAME": _TAG,
            "IS_PRERELEASE": "true",
            **(overrides or {}),
        },
    )
    completed = run_bash(step_script(*step), tmp_path, environment)
    recorded = (
        [json.loads(line) for line in calls.read_text(encoding="utf-8").splitlines()]
        if calls.exists()
        else []
    )
    return completed, recorded


@pytest.mark.parametrize("release_state", ["draft", "published"])
def test_an_existing_draft_or_published_release_is_reused(
    tmp_path: Path, release_state: str
) -> None:
    """A re-run accepts either release state and creates nothing new."""
    completed, calls = _run_gh_step(
        tmp_path,
        _DRAFT_STEP,
        overrides={"RELEASE_STATE": release_state, "IS_PRERELEASE": "true"},
    )
    assert completed.returncode == 0, completed.stderr
    assert calls == [
        [
            "api",
            "--paginate",
            "--slurp",
            "repos/leynos/femtologging/releases",
        ]
    ], "A re-run accepts either release state and creates nothing new."


@pytest.mark.parametrize(
    ("prerelease", "has_prerelease_flag"), [("true", True), ("false", False)]
)
def test_a_missing_release_is_created_as_a_draft(
    tmp_path: Path, prerelease: str, *, has_prerelease_flag: bool
) -> None:
    """A new release starts as a verified draft with the correct prerelease bit."""
    completed, calls = _run_gh_step(
        tmp_path,
        _DRAFT_STEP,
        overrides={"IS_PRERELEASE": prerelease},
    )
    assert completed.returncode == 0, completed.stderr
    assert len(calls) == 2, (
        "A new release starts as a verified draft with the correct prerelease bit."
    )
    create = calls[1]
    assert create[:3] == ["release", "create", _TAG], (
        "A new release starts as a verified draft with the correct prerelease bit."
    )
    assert {"--draft", "--verify-tag", "--generate-notes"} <= set(create), (
        "A new release starts as a verified draft with the correct prerelease bit."
    )
    assert ("--prerelease" in create) is has_prerelease_flag, (
        "A new release starts as a verified draft with the correct prerelease bit."
    )


def test_duplicate_release_tags_fail_without_creating_another_draft(
    tmp_path: Path,
) -> None:
    """Duplicate matching releases stop the workflow before creation."""
    completed, calls = _run_gh_step(
        tmp_path,
        _DRAFT_STEP,
        overrides={"RELEASE_STATE": "draft", "RELEASE_COUNT": "2"},
    )

    assert completed.returncode != 0, (
        "Duplicate releases for the tag must stop draft creation."
    )
    assert "Multiple GitHub Releases use tag" in completed.stderr, (
        "Duplicate releases for the tag must explain how to recover."
    )
    assert len(calls) == 1, "Duplicate releases must not trigger a create call."
    assert calls[0][0] == "api", "The release list must detect duplicate tags."


def test_release_list_errors_fail_without_creating_a_draft(
    tmp_path: Path,
) -> None:
    """A failed GitHub lookup cannot be mistaken for an absent release."""
    completed, calls = _run_gh_step(
        tmp_path,
        _DRAFT_STEP,
        overrides={"GH_RELEASE_LIST_STATUS": "1"},
    )

    assert completed.returncode != 0, (
        "A failed release lookup must stop the draft step."
    )
    assert len(calls) == 1, "A failed lookup must not trigger release creation."
    assert calls[0][0] == "api", "The release list request must fail safely."


def test_snapshot_downloads_only_assets_needed_for_pypi_carryover(
    tmp_path: Path,
) -> None:
    """The draft snapshot carries a matching GitHub asset's existing bytes."""
    publish = tmp_path / "dist/publish"
    publish.mkdir(parents=True)
    (publish / "femtologging.whl").write_bytes(b"rebuilt wheel")
    (publish / "femtologging.tar.gz").write_bytes(b"rebuilt sdist")
    digest = hashlib.sha256(b"canonical GitHub sdist").hexdigest()
    assets_json = tmp_path / "fake-assets.json"
    assets_json.write_text(
        json.dumps({
            "assets": [
                {"name": "femtologging.tar.gz", "digest": f"sha256:{digest}"},
                {"name": "old-release.tar.gz", "digest": f"sha256:{digest}"},
            ]
        }),
        encoding="utf-8",
    )
    (tmp_path / "fake-assets").mkdir()
    (tmp_path / "fake-assets/femtologging.tar.gz").write_bytes(
        b"canonical GitHub sdist"
    )

    completed, calls = _run_gh_step(tmp_path, _SNAPSHOT_STEP)

    assert completed.returncode == 0, completed.stderr
    carried = tmp_path / "github-state/files/femtologging.tar.gz"
    assert carried.read_bytes() == b"canonical GitHub sdist", (
        "The draft snapshot carries a matching GitHub asset's existing bytes."
    )
    assert sorted(
        path.name for path in (tmp_path / "github-state/files").iterdir()
    ) == ["femtologging.tar.gz"], (
        "The draft snapshot carries a matching GitHub asset's existing bytes."
    )
    assert [call[:2] for call in calls] == [
        ["release", "view"],
        ["release", "download"],
    ], "The draft snapshot carries a matching GitHub asset's existing bytes."


def test_existing_pypi_and_github_digest_disagreement_blocks_staging(
    tmp_path: Path,
) -> None:
    """An existing GitHub filename with different PyPI bytes blocks the job."""
    publish = tmp_path / "dist/publish"
    publish.mkdir(parents=True)
    (publish / "femtologging.whl").write_bytes(b"new local build")
    github_files = tmp_path / "github-files"
    github_files.mkdir()
    github_bytes = b"old GitHub bytes"
    (github_files / "femtologging.whl").write_bytes(github_bytes)
    github_digest = hashlib.sha256(github_bytes).hexdigest()
    index_path = tmp_path / "pypi-state/pypi-index.json"
    index_path.parent.mkdir(parents=True)
    index_path.write_text(
        json.dumps({
            "files": [
                {
                    "filename": "femtologging.whl",
                    "hashes": {
                        "sha256": hashlib.sha256(b"canonical PyPI bytes").hexdigest()
                    },
                    "url": "https://files.test/fem.whl",
                }
            ]
        }),
        encoding="utf-8",
    )
    assets_path = tmp_path / "release-assets.json"
    assets_path.write_text(
        json.dumps({
            "assets": [
                {
                    "name": "femtologging.whl",
                    "digest": f"sha256:{github_digest}",
                }
            ]
        }),
        encoding="utf-8",
    )
    (tmp_path / "dist/attestations").mkdir(parents=True)

    completed, _ = _run_gh_step(
        tmp_path,
        _STAGE_STEP,
        overrides={
            "PUBLISH_DIRECTORY": "dist/publish",
            "PYPI_INDEX_PATH": "pypi-state/pypi-index.json",
            "GITHUB_ASSETS_PATH": "release-assets.json",
            "GITHUB_FILES": "github-files",
            "ATTESTATION_DIRECTORY": "dist/attestations",
            "GITHUB_UPLOAD_DIRECTORY": "github-upload",
            "GITHUB_OUTPUT_PATH": "pypi-fetch.tsv",
        },
    )

    assert completed.returncode != 0, (
        "An existing GitHub filename with different PyPI bytes blocks the job."
    )
    assert "differs between PyPI and the GitHub Release" in completed.stderr, (
        "An existing GitHub filename with different PyPI bytes blocks the job."
    )
    assert not (tmp_path / "github-output").exists(), (
        "a digest mismatch must not report the stage as safe to upload"
    )


def test_upload_step_sends_only_staged_assets_without_clobber(
    tmp_path: Path,
) -> None:
    """The gh upload receives only staged paths and never uses `--clobber`."""
    upload_directory = tmp_path / "github-upload"
    upload_directory.mkdir()
    staged = upload_directory / "femtologging-new.whl"
    staged.write_bytes(b"canonical package bytes")

    completed, calls = _run_gh_step(
        tmp_path,
        _UPLOAD_STEP,
        overrides={"GITHUB_UPLOAD_DIRECTORY": "github-upload"},
    )

    assert completed.returncode == 0, completed.stderr
    assert len(calls) == 1, (
        "The gh upload receives only staged paths and never uses `--clobber`."
    )
    assert calls[0][:3] == ["release", "upload", _TAG], (
        "The gh upload receives only staged paths and never uses `--clobber`."
    )
    assert calls[0][3:] == [str(staged.relative_to(tmp_path))], (
        "The gh upload receives only staged paths and never uses `--clobber`."
    )
    assert "--clobber" not in calls[0], (
        "The gh upload receives only staged paths and never uses `--clobber`."
    )


@pytest.mark.parametrize("prerelease", ["true", "false"])
def test_publication_preserves_the_prerelease_flag(
    tmp_path: Path, prerelease: str
) -> None:
    """The final draft edit gets the version check's prerelease result."""
    completed, calls = _run_gh_step(
        tmp_path,
        _PUBLISH_STEP,
        overrides={"IS_PRERELEASE": prerelease},
    )
    assert completed.returncode == 0, completed.stderr
    assert calls == [
        ["release", "edit", _TAG, "--draft=false", f"--prerelease={prerelease}"]
    ], "The final draft edit gets the version check's prerelease result."
