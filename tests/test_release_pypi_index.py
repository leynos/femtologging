"""Exercise PyPI Simple index fetching in the release workflow."""

from __future__ import annotations

import dataclasses
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
    import pathlib
    import subprocess

RELEASE_WORKFLOW = ".github/workflows/release.yml"
_INDEX_STEP = (RELEASE_WORKFLOW, "publish-pypi", "Fetch the PyPI Simple index")

pytestmark = pytest.mark.skipif(
    sys.platform == "win32",
    reason="The release workflow's shell steps run under Bash on Linux.",
)


@dataclasses.dataclass(frozen=True, slots=True)
class IndexScenario:
    """Describe the fake index responses and files expected after upload."""

    statuses: str
    allow_missing: str = "true"
    expected_names: tuple[str, ...] | None = None
    response_names: tuple[tuple[str, ...], ...] = ()


@dataclasses.dataclass(frozen=True, slots=True)
class IndexRunLayout:
    """Hold scratch paths used by one fake PyPI index run."""

    tools: pathlib.Path
    calls: pathlib.Path
    arguments: pathlib.Path
    sleeps: pathlib.Path
    responses: pathlib.Path
    expected_directory: pathlib.Path


def _fake_curl_tools(directory: pathlib.Path) -> pathlib.Path:
    """Install a scripted curl and no-op sleep tool for index tests."""
    tools = directory / "fake-tools"
    curl_body = (
        "import json, os, pathlib, sys\n"
        "args = sys.argv[1:]\n"
        "calls = pathlib.Path(os.environ['FAKE_CURL_CALLS'])\n"
        "index = int(calls.read_text() or '0') if calls.exists() else 0\n"
        "calls.write_text(str(index + 1))\n"
        "arguments = pathlib.Path(os.environ['FAKE_CURL_ARGUMENTS'])\n"
        "arguments.write_text(json.dumps(args), encoding='utf-8')\n"
        "statuses = os.environ['FAKE_CURL_STATUSES'].split(',')\n"
        "status = statuses[min(index, len(statuses) - 1)]\n"
        "output = pathlib.Path(args[args.index('--output') + 1])\n"
        "if status != '000':\n"
        "    response_dir = os.environ.get('FAKE_CURL_RESPONSE_DIRECTORY')\n"
        "    response = (\n"
        "        pathlib.Path(response_dir, str(index))\n"
        "        if response_dir else None\n"
        "    )\n"
        "    body = (\n"
        "        response.read_text(encoding='utf-8')\n"
        "        if response and response.exists()\n"
        "        else os.environ['FAKE_CURL_BODY']\n"
        "    )\n"
        "    output.write_text(body, encoding='utf-8')\n"
        "print(status, end='')\n"
        "raise SystemExit(7 if status == '000' else 0)\n"
    )
    write_argument_recorder(tools, "curl", curl_body)
    sleep = tools / "sleep"
    sleep.write_text(
        "#!/usr/bin/env bash\n"
        "set -euo pipefail\n"
        'printf \'%s\\n\' "$*" >> "${FAKE_SLEEP_CALLS}"\n',
        encoding="utf-8",
    )
    sleep.chmod(0o755)
    return tools


def _prepare_index_run(
    tmp_path: pathlib.Path, scenario: IndexScenario
) -> IndexRunLayout:
    """Create fake index responses and the expected upload directory."""
    layout = IndexRunLayout(
        tools=_fake_curl_tools(tmp_path),
        calls=tmp_path / "curl-calls",
        arguments=tmp_path / "curl-arguments",
        sleeps=tmp_path / "sleep-calls",
        responses=tmp_path / "curl-responses",
        expected_directory=tmp_path / "dist/publish",
    )
    if scenario.response_names:
        layout.responses.mkdir()
        for index, names in enumerate(scenario.response_names):
            body = json.dumps({"files": [{"filename": name} for name in names]})
            (layout.responses / str(index)).write_text(body, encoding="utf-8")
    layout.expected_directory.mkdir(parents=True)
    for name in scenario.expected_names or ():
        (layout.expected_directory / name).write_bytes(name.encode("utf-8"))
    return layout


def _index_environment(
    tmp_path: pathlib.Path, scenario: IndexScenario, layout: IndexRunLayout
) -> dict[str, str]:
    """Build environment values for a fake PyPI Simple index request."""
    path = os.pathsep.join((str(layout.tools), os.environ.get("PATH", os.defpath)))
    overrides = {
        "PATH": path,
        "FAKE_CURL_STATUSES": scenario.statuses,
        "FAKE_CURL_BODY": '{"files": []}',
        "FAKE_CURL_CALLS": str(layout.calls),
        "FAKE_CURL_ARGUMENTS": str(layout.arguments),
        "FAKE_SLEEP_CALLS": str(layout.sleeps),
        "PYPI_ALLOW_MISSING": scenario.allow_missing,
        "PYPI_EXPECTED_DIRECTORY": str(layout.expected_directory),
    }
    if scenario.response_names:
        overrides["FAKE_CURL_RESPONSE_DIRECTORY"] = str(layout.responses)
    return workflow_command_environment(_INDEX_STEP, tmp_path, overrides)


def _run_index(
    tmp_path: pathlib.Path, scenario: IndexScenario
) -> tuple[subprocess.CompletedProcess[str], int, list[str]]:
    """Run the checked-in index-fetch step with the given fake responses."""
    layout = _prepare_index_run(tmp_path, scenario)
    completed = run_bash(
        step_script(*_INDEX_STEP),
        tmp_path,
        _index_environment(tmp_path, scenario, layout),
    )
    curl_count = (
        int(layout.calls.read_text(encoding="utf-8")) if layout.calls.exists() else 0
    )
    sleep_values = (
        layout.sleeps.read_text(encoding="utf-8").splitlines()
        if layout.sleeps.exists()
        else []
    )
    return completed, curl_count, sleep_values


def test_a_missing_pypi_project_becomes_an_empty_index(tmp_path: pathlib.Path) -> None:
    """A 404 before the first release means there is nothing to skip."""
    completed, calls, sleeps = _run_index(tmp_path, IndexScenario(statuses="404"))
    assert completed.returncode == 0, completed.stderr
    assert json.loads((tmp_path / "pypi-index.json").read_text(encoding="utf-8")) == {
        "files": []
    }, "A 404 before the first release means there is nothing to skip."
    assert (calls, sleeps) == (1, []), (
        "A 404 before the first release means there is nothing to skip."
    )


def test_pypi_index_fetch_restricts_redirects_to_https(tmp_path: pathlib.Path) -> None:
    """The index request passes HTTPS-only protocol restrictions to curl."""
    completed, calls, _ = _run_index(tmp_path, IndexScenario(statuses="200"))
    assert completed.returncode == 0, completed.stderr
    assert calls == 1, "A successful index fetch makes one curl request."
    arguments = json.loads((tmp_path / "curl-arguments").read_text(encoding="utf-8"))
    assert arguments[arguments.index("--proto") + 1] == "=https", (
        "The index request permits only HTTPS as its initial protocol."
    )
    assert arguments[arguments.index("--proto-redir") + 1] == "=https", (
        "The index request permits only HTTPS redirects."
    )


def test_transient_index_failures_retry_with_bounded_two_second_waits(
    tmp_path: pathlib.Path,
) -> None:
    """Network errors, 429, and 5xx retry until a successful response."""
    completed, calls, sleeps = _run_index(
        tmp_path,
        IndexScenario(statuses="000,429,503,200"),
    )
    assert completed.returncode == 0, completed.stderr
    assert (calls, sleeps) == (4, ["2", "2", "2"]), (
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
    tmp_path: pathlib.Path, status: str, expected_calls: int
) -> None:
    """Persistent transient errors and other HTTP failures stop publication."""
    completed, calls, _ = _run_index(tmp_path, IndexScenario(statuses=status))
    assert completed.returncode != 0, (
        "Persistent transient errors and other HTTP failures stop publication."
    )
    assert f"PyPI Simple index returned HTTP {status}." in completed.stderr, (
        "Persistent transient errors and other HTTP failures stop publication."
    )
    assert calls == expected_calls, (
        "Persistent transient errors and other HTTP failures stop publication."
    )


def test_index_polling_waits_for_every_uploaded_filename(
    tmp_path: pathlib.Path,
) -> None:
    """A stale successful index is refreshed until the upload is visible."""
    names = ("femtologging-1.0.0.whl", "femtologging-1.0.0.tar.gz")
    completed, calls, sleeps = _run_index(
        tmp_path,
        IndexScenario(
            statuses="200,200",
            allow_missing="false",
            expected_names=names,
            response_names=((), names),
        ),
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert (calls, sleeps) == (2, ["3"]), (
        "A stale successful index is refreshed until the upload is visible."
    )


def test_index_polling_ignores_pypi_publish_attestation_sidecars(
    tmp_path: pathlib.Path,
) -> None:
    """PyPI index polling waits for files PyPI actually lists."""
    distribution = "femtologging-1.0.0.whl"
    completed, calls, sleeps = _run_index(
        tmp_path,
        IndexScenario(
            statuses="200",
            allow_missing="false",
            expected_names=(distribution, f"{distribution}.publish.attestation"),
            response_names=((distribution,),),
        ),
    )

    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert (calls, sleeps) == (1, []), (
        "PyPI-only publish attestations must not delay index reconciliation."
    )


def test_index_polling_retries_an_unpublished_project_404(
    tmp_path: pathlib.Path,
) -> None:
    """A new project 404 is treated as empty until the upload appears."""
    name = "femtologging-1.0.0.whl"
    completed, calls, sleeps = _run_index(
        tmp_path,
        IndexScenario(
            statuses="404,200",
            expected_names=(name,),
            response_names=((), (name,)),
        ),
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert (calls, sleeps) == (2, ["3"]), (
        "A new project 404 is treated as empty until the upload appears."
    )


def test_index_polling_fails_with_names_still_missing_after_ten_checks(
    tmp_path: pathlib.Path,
) -> None:
    """A persistent stale index fails with the uploaded filenames listed."""
    name = "femtologging-1.0.0.whl"
    completed, calls, sleeps = _run_index(
        tmp_path,
        IndexScenario(statuses="200", allow_missing="false", expected_names=(name,)),
    )
    assert completed.returncode != 0, (
        "A persistent stale index fails with the uploaded filenames listed."
    )
    assert calls == 10, "A persistent stale index fails after ten index checks."
    assert sleeps == ["3"] * 9, "Polling waits three seconds between checks."
    assert "did not list these published files after 10 checks" in completed.stderr, (
        "A persistent stale index fails after ten index checks."
    )
    assert name in completed.stderr, (
        "A persistent stale index fails with the uploaded filenames listed."
    )


def test_index_polling_skips_when_no_files_were_uploaded(
    tmp_path: pathlib.Path,
) -> None:
    """A rerun with no missing uploads does not wait for index propagation."""
    completed, calls, sleeps = _run_index(
        tmp_path,
        IndexScenario(statuses="200", allow_missing="false", expected_names=()),
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert (calls, sleeps) == (1, []), (
        "A rerun with no missing uploads does not wait for index propagation."
    )
