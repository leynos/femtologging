"""Execute the checked-in release version, collection, and PyPI steps."""

from __future__ import annotations

import hashlib
import json
import os
import sys
import typing as typ

import pytest

from tests.make_contract_helpers import repo_root
from tests.release_workflow_helpers import (
    output_values,
    run_bash,
    step_script,
    workflow_command_environment,
    write_argument_recorder,
)

if typ.TYPE_CHECKING:
    import subprocess
    from pathlib import Path

RELEASE_WORKFLOW = ".github/workflows/release.yml"
_VERSION_STEP = (
    RELEASE_WORKFLOW,
    "check-version",
    "Check the Python project version against the tag",
)
_COLLECT_STEP = (
    RELEASE_WORKFLOW,
    "attest",
    "Collect every wheel and exactly one source distribution",
)
_INDEX_STEP = (RELEASE_WORKFLOW, "publish-pypi", "Fetch the PyPI Simple index")
_STAGE_STEP = (RELEASE_WORKFLOW, "publish-pypi", "Skip filenames already held by PyPI")

pytestmark = pytest.mark.skipif(
    sys.platform == "win32",
    reason="The release workflow's shell steps run under Bash on Linux.",
)


def _run_step(
    tmp_path: Path,
    step: tuple[str, str, str],
    overrides: dict[str, str],
) -> subprocess.CompletedProcess[str]:
    """Run one checked-in shell script in an isolated workspace."""
    environment = workflow_command_environment(step, tmp_path, overrides)
    return run_bash(step_script(*step), tmp_path, environment)


def _write_project(path: Path, version: str) -> None:
    """Write the minimal project metadata read by the version step."""
    path.write_text(f'[project]\nversion = "{version}"\n', encoding="utf-8")


def _run_version(
    tmp_path: Path, declared: str, tag_name: str
) -> tuple[subprocess.CompletedProcess[str], dict[str, str]]:
    """Run the real version step against a temporary pyproject file."""
    project = tmp_path / "pyproject.toml"
    _write_project(project, declared)
    completed = _run_step(
        tmp_path,
        _VERSION_STEP,
        {"TAG_NAME": tag_name, "PYPROJECT_PATH": str(project)},
    )
    return completed, output_values(tmp_path / "github-output")


@pytest.mark.parametrize(
    ("version", "prerelease"),
    [
        ("0.2.0-beta1", "true"),
        ("0.2.0b1", "true"),
        ("1.0.0-rc.2", "true"),
        ("1.0.0a3", "true"),
        ("1.2.3", "false"),
        ("1.2.3.post1", "false"),
        ("1.2.3+rc1", "false"),
    ],
)
def test_a_matching_tag_reports_prerelease_status(
    tmp_path: Path, version: str, prerelease: str
) -> None:
    """A `v` tag matches only its exact project version string."""
    completed, outputs = _run_version(tmp_path, version, f"v{version}")
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert outputs == {"prerelease": prerelease}, (
        "A `v` tag matches only its exact project version string."
    )


@pytest.mark.parametrize(
    ("declared", "tag"),
    [
        ("1.2.2", "v1.2.3"),
        ("0.2.0-beta1", "v0.2.0b1"),
        ("0.2.0-beta1", "v0.2.0"),
        ("0.2.0-beta1", "vv0.2.0-beta1"),
    ],
)
def test_a_mismatched_tag_fails_before_publication(
    tmp_path: Path, declared: str, tag: str
) -> None:
    """Respelled or mismatched versions produce no prerelease output."""
    completed, outputs = _run_version(tmp_path, declared, tag)
    assert completed.returncode != 0, (
        "Respelled or mismatched versions produce no prerelease output."
    )
    assert "::error file=pyproject.toml,title=Tag/pyproject.toml mismatch::" in (
        completed.stdout
    ), "Respelled or mismatched versions produce no prerelease output."
    assert not outputs, "Respelled or mismatched versions produce no prerelease output."


def test_repository_manifests_declare_the_same_version() -> None:
    """A tag must be able to satisfy both the Cargo and Python checks."""
    import tomllib

    root = repo_root()
    pyproject = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8"))
    cargo = tomllib.loads(
        (root / "rust_extension/Cargo.toml").read_text(encoding="utf-8")
    )
    assert pyproject["project"]["version"] == cargo["package"]["version"], (
        "A tag must be able to satisfy both the Cargo and Python checks."
    )


def _add_artifact(directory: Path, name: str) -> None:
    """Create one artifact file in the given downloaded-artifact directory."""
    directory.mkdir(parents=True, exist_ok=True)
    (directory / name).write_bytes(name.encode("utf-8"))


def test_collection_copies_all_wheels_and_exactly_one_sdist(tmp_path: Path) -> None:
    """Every native wheel and the sole sdist are put in the publish folder."""
    _add_artifact(tmp_path / "dist", "femtologging-cp312-linux.whl")
    _add_artifact(tmp_path / "dist", "femtologging-cp313-macos.whl")
    _add_artifact(tmp_path / "dist", "femtologging-0.1.0.tar.gz")

    completed = _run_step(tmp_path, _COLLECT_STEP, {})

    assert completed.returncode == 0, completed.stderr
    names = sorted(path.name for path in (tmp_path / "dist/publish").iterdir())
    assert names == [
        "femtologging-0.1.0.tar.gz",
        "femtologging-cp312-linux.whl",
        "femtologging-cp313-macos.whl",
    ], "Every native wheel and the sole sdist are put in the publish folder."


@pytest.mark.parametrize(
    ("sdists", "message"),
    [
        ((), "Expected exactly one source distribution, found none."),
        (
            ("femtologging-0.1.tar.gz", "femtologging-0.2.tar.gz"),
            "Expected exactly one source distribution, found multiple.",
        ),
    ],
)
def test_collection_refuses_zero_or_multiple_sdists(
    tmp_path: Path, sdists: tuple[str, ...], message: str
) -> None:
    """Missing and duplicate sdists stop the attestation job."""
    _add_artifact(tmp_path / "dist", "femtologging-cp312-linux.whl")
    for name in sdists:
        _add_artifact(tmp_path / "dist", name)

    completed = _run_step(tmp_path, _COLLECT_STEP, {})

    assert completed.returncode != 0, (
        "Missing and duplicate sdists stop the attestation job."
    )
    assert message in completed.stderr, (
        "Missing and duplicate sdists stop the attestation job."
    )


def _fake_curl_tools(directory: Path, statuses: str, body: str) -> tuple[Path, Path]:
    """Install scripted curl and no-op sleep tools for index lookup tests."""
    tools = directory / "fake-tools"
    curl_body = (
        "import os, pathlib, sys\n"
        "args = sys.argv[1:]\n"
        "calls = pathlib.Path(os.environ['FAKE_CURL_CALLS'])\n"
        "index = int(calls.read_text() or '0') if calls.exists() else 0\n"
        "calls.write_text(str(index + 1))\n"
        "statuses = os.environ['FAKE_CURL_STATUSES'].split(',')\n"
        "status = statuses[min(index, len(statuses) - 1)]\n"
        "output = pathlib.Path(args[args.index('--output') + 1])\n"
        "if status != '000':\n"
        "    output.write_text(os.environ['FAKE_CURL_BODY'], encoding='utf-8')\n"
        "print(status, end='')\n"
        "raise SystemExit(7 if status == '000' else 0)\n"
    )
    curl = write_argument_recorder(tools, "curl", curl_body)
    sleep = tools / "sleep"
    sleep.write_text(
        "#!/usr/bin/env bash\n"
        "set -euo pipefail\n"
        'printf \'%s\\n\' "$*" >> "${FAKE_SLEEP_CALLS}"\n',
        encoding="utf-8",
    )
    sleep.chmod(0o755)
    return tools, curl


def _run_index(
    tmp_path: Path, statuses: str, *, allow_missing: str = "true"
) -> tuple[subprocess.CompletedProcess[str], int, int]:
    """Run the checked-in index-fetch step with a response sequence."""
    tools, _ = _fake_curl_tools(tmp_path, statuses, '{"files": []}')
    calls = tmp_path / "curl-calls"
    sleeps = tmp_path / "sleep-calls"
    path = os.pathsep.join((str(tools), os.environ.get("PATH", os.defpath)))
    environment = workflow_command_environment(
        _INDEX_STEP,
        tmp_path,
        {
            "PATH": path,
            "FAKE_CURL_STATUSES": statuses,
            "FAKE_CURL_BODY": '{"files": []}',
            "FAKE_CURL_CALLS": str(calls),
            "FAKE_SLEEP_CALLS": str(sleeps),
            "PYPI_ALLOW_MISSING": allow_missing,
        },
    )
    completed = run_bash(step_script(*_INDEX_STEP), tmp_path, environment)
    curl_count = int(calls.read_text(encoding="utf-8")) if calls.exists() else 0
    sleep_count = (
        len(sleeps.read_text(encoding="utf-8").splitlines()) if sleeps.exists() else 0
    )
    return completed, curl_count, sleep_count


def test_a_missing_pypi_project_becomes_an_empty_index(tmp_path: Path) -> None:
    """A 404 before the first release means there is nothing to skip."""
    completed, calls, sleeps = _run_index(tmp_path, "404")
    assert completed.returncode == 0, completed.stderr
    assert json.loads((tmp_path / "pypi-index.json").read_text(encoding="utf-8")) == {
        "files": []
    }, "A 404 before the first release means there is nothing to skip."
    assert (calls, sleeps) == (1, 0), (
        "A 404 before the first release means there is nothing to skip."
    )


def test_transient_index_failures_retry_with_bounded_two_second_waits(
    tmp_path: Path,
) -> None:
    """Network errors, 429, and 5xx retry until a successful response."""
    completed, calls, sleeps = _run_index(tmp_path, "000,429,503,200")
    assert completed.returncode == 0, completed.stderr
    assert (calls, sleeps) == (4, 3), (
        "Network errors, 429, and 5xx retry until a successful response."
    )
    assert (tmp_path / "pypi-index.json").read_text(
        encoding="utf-8"
    ) == '{"files": []}', (
        "Network errors, 429, and 5xx retry until a successful response."
    )


@pytest.mark.parametrize(
    ("status", "expected_calls"),
    [("500", 6), ("000", 6), ("403", 1)],
)
def test_final_index_http_errors_fail(
    tmp_path: Path, status: str, expected_calls: int
) -> None:
    """Persistent transient errors and other HTTP failures stop publication."""
    completed, calls, _ = _run_index(tmp_path, status)
    assert completed.returncode != 0, (
        "Persistent transient errors and other HTTP failures stop publication."
    )
    assert f"PyPI Simple index returned HTTP {status}." in completed.stderr, (
        "Persistent transient errors and other HTTP failures stop publication."
    )
    assert calls == expected_calls, (
        "Persistent transient errors and other HTTP failures stop publication."
    )


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


def _run_stage_pypi(
    tmp_path: Path, published: tuple[str, ...]
) -> tuple[subprocess.CompletedProcess[str], dict[str, str], list[str]]:
    """Run the checked-in PyPI skip step in a scratch release layout."""
    publish = tmp_path / "dist/publish"
    publish.mkdir(parents=True)
    names = ("femtologging.whl", "femtologging.tar.gz")
    for name in names:
        (publish / name).write_bytes(f"rebuilt:{name}".encode())
    _write_index(tmp_path / "pypi-index.json", published)
    (tmp_path / "github-state/files").mkdir(parents=True)
    (tmp_path / "github-state/assets.json").write_text('{"assets": []}', "utf-8")

    completed = _run_step(tmp_path, _STAGE_STEP, {})
    outputs = output_values(tmp_path / "github-output")
    remaining = sorted(path.name for path in publish.iterdir())
    return completed, outputs, remaining


def test_pypi_skip_keeps_only_names_that_are_not_already_published(
    tmp_path: Path,
) -> None:
    """The skip step leaves only missing names and reports an upload is needed."""
    completed, outputs, remaining = _run_stage_pypi(tmp_path, ("femtologging.whl",))
    assert completed.returncode == 0, completed.stderr
    assert remaining == ["femtologging.tar.gz"], (
        "The skip step leaves only missing names and reports an upload is needed."
    )
    assert outputs == {"remaining": "true"}, (
        "The skip step leaves only missing names and reports an upload is needed."
    )


def test_pypi_skip_succeeds_when_nothing_remains_to_upload(tmp_path: Path) -> None:
    """A re-run with every filename on PyPI reports remaining=false."""
    names = ("femtologging.whl", "femtologging.tar.gz")
    completed, outputs, remaining = _run_stage_pypi(tmp_path, names)
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert remaining == [], (
        "A re-run with every filename on PyPI reports remaining=false."
    )
    assert outputs == {"remaining": "false"}, (
        "A re-run with every filename on PyPI reports remaining=false."
    )
