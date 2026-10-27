"""Exercise the reusable wheel workflow's source-distribution count check."""

from __future__ import annotations

import sys
import typing as typ

import pytest
import yaml

from tests.make_contract_helpers import (
    mapping,
    objects,
    repo_root,
    sole_workflow_step,
    workflow_job,
)
from tests.release_workflow_helpers import run_bash, step_environment

if typ.TYPE_CHECKING:
    from pathlib import Path

WORKFLOW = ".github/workflows/build-wheels.yml"
JOB = "verify-wheel-install"
STEP = "Check source distribution cardinality"

pytestmark = pytest.mark.skipif(
    sys.platform == "win32",
    reason="The checked-in cardinality step runs with Bash on Linux.",
)


def _run_with_count(tmp_path: Path, count: int) -> tuple[int, str, str]:
    """Run the checked-in sdist count step with ``count`` tarballs."""
    directory = tmp_path / "dist"
    directory.mkdir()
    (directory / "femtologging-cp312-manylinux_x86_64.whl").touch()
    for index in range(count):
        (directory / f"femtologging-0.1.{index}.tar.gz").touch()
    environment = {
        **step_environment(WORKFLOW, JOB, STEP),
        "GITHUB_OUTPUT": str(tmp_path / "github-output"),
    }
    result = run_bash(
        str(sole_workflow_step(WORKFLOW, JOB, STEP)["run"]), tmp_path, environment
    )
    return result.returncode, result.stdout, result.stderr


def test_zero_sdists_fails_with_the_fixed_message(tmp_path: Path) -> None:
    """A missing sdist must fail in pull-request CI."""
    code, _, stderr = _run_with_count(tmp_path, 0)
    assert code != 0, "A missing sdist must fail in pull-request CI."
    assert "Expected exactly one sdist, found none." in stderr, (
        "A missing sdist must fail in pull-request CI."
    )


def test_one_sdist_passes(tmp_path: Path) -> None:
    """A single sdist is accepted alongside native wheels."""
    code, stdout, stderr = _run_with_count(tmp_path, 1)
    assert code == 0, f"{stdout}{stderr}"


def test_two_sdists_fail_with_the_fixed_message(tmp_path: Path) -> None:
    """Duplicate sdists must fail in pull-request CI."""
    code, _, stderr = _run_with_count(tmp_path, 2)
    assert code != 0, "Duplicate sdists must fail in pull-request CI."
    assert "Expected exactly one sdist, found multiple." in stderr, (
        "Duplicate sdists must fail in pull-request CI."
    )


def test_sdist_builds_share_the_pinned_maturin_constraint() -> None:
    """The sdist and rebuild constrain maturin to the wheel workflow's pin."""
    workflow = mapping(
        yaml.safe_load((repo_root() / WORKFLOW).read_text(encoding="utf-8")),
        subject=WORKFLOW,
    )
    maturin_version = mapping(workflow.get("env"), subject="workflow env").get(
        "MATURIN_VERSION"
    )
    assert isinstance(maturin_version, str), "The workflow pins maturin."
    constraints_path = ".github/maturin-build-constraints.txt"
    assert (repo_root() / constraints_path).read_text(encoding="utf-8") == (
        f"maturin=={maturin_version}\n"
    ), "Both isolated sdist builds use the wheel workflow's maturin pin."
    expected_constraint = f"${{{{ github.workspace }}}}/{constraints_path}"

    sdist = sole_workflow_step(WORKFLOW, "build-linux", "Build the source distribution")
    sdist_environment = mapping(sdist.get("env"), subject="sdist build env")
    assert sdist_environment.get("UV_BUILD_CONSTRAINT") == expected_constraint, (
        "Both isolated sdist builds use the wheel workflow's maturin pin."
    )

    rebuild = sole_workflow_step(
        WORKFLOW,
        JOB,
        "Rebuild the sdist and install it in a fresh environment",
    )
    rebuild_environment = mapping(rebuild.get("env"), subject="sdist rebuild env")
    assert rebuild_environment.get("UV_BUILD_CONSTRAINT") == expected_constraint, (
        "Both isolated sdist builds use the wheel workflow's maturin pin."
    )
    rebuild_script = str(rebuild.get("run", ""))
    assert "uv build --wheel" in rebuild_script, (
        "The sdist is rebuilt with uv in its isolated build environment."
    )
    assert "uv pip install --python" in rebuild_script, (
        "The rebuilt wheel is installed into the fresh verification environment."
    )


def test_sdist_build_installs_the_pinned_rust_toolchain() -> None:
    """The host-side Maturin backend uses the repository toolchain pin."""
    steps = objects(
        workflow_job(WORKFLOW, "build-linux").get("steps"),
        subject="build-linux steps",
    )
    setup = sole_workflow_step(
        WORKFLOW, "build-linux", "Install the pinned Rust toolchain for the sdist"
    )
    assert setup.get("uses") == (
        "leynos/shared-actions/.github/actions/setup-rust@"
        "ff1dd759dfffc0db3459e30e833f52437ee62b57"
    ), "The sdist builder installs the repository-pinned Rust toolchain."
    assert setup.get("if") == "matrix.arch == 'x86_64'", (
        "Only the Linux x86_64 leg builds the sdist."
    )

    step_names = [step.get("name") for step in steps]
    setup_index = step_names.index("Install the pinned Rust toolchain for the sdist")
    sdist_index = step_names.index("Build the source distribution")
    assert setup_index < sdist_index, (
        "The pinned Rust toolchain is available before the sdist build."
    )
