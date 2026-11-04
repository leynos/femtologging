"""Contract tests for the pre-upload probes of the release wheels.

A wheel that cannot install or import must never be published, so every wheel
build probes the wheels it just produced before it uploads them. These tests
pin that ordering, the interpreter coverage, and the isolation each probe runs
in.
"""

from __future__ import annotations

import re
import typing as typ

import pytest
import yaml

from tests.make_contract_helpers import mapping, objects, repo_root, workflow_job

if typ.TYPE_CHECKING:
    from pathlib import Path

ROOT = repo_root()
WORKFLOW = ".github/workflows/build-wheels.yml"
WHEEL_JOBS = ("build-linux", "build-other")
PROBE_SCRIPTS = (
    ROOT / "scripts" / "probe_host_wheels.sh",
    ROOT / "scripts" / "probe_manylinux_wheels.sh",
)
INTERPRETERS = re.compile(r"--interpreter\s+([\d.]+(?:\s+[\d.]+)*)")


def _steps(job_name: str) -> list[dict[str, object]]:
    """Return the steps of one wheel-building job."""
    job = workflow_job(WORKFLOW, job_name)
    return objects(job.get("steps"), subject=f"{job_name} steps")


def _is_wheel_build(step: dict[str, object]) -> bool:
    """Return whether the step builds wheels with maturin."""
    return str(step.get("uses", "")).startswith("messense/maturin-action@")


def _is_wheel_upload(step: dict[str, object]) -> bool:
    """Return whether the step uploads the built wheels."""
    if not str(step.get("uses", "")).startswith("actions/upload-artifact@"):
        return False
    inputs = mapping(step.get("with"), subject="upload-artifact inputs")
    return inputs.get("path") == "dist/*.whl"


def _probe_step(job_name: str) -> dict[str, object]:
    """Return the job's single wheel probe, which runs a checked-in script."""
    probes = [
        step
        for step in _steps(job_name)
        if "scripts/probe_" in str(step.get("run", ""))
    ]
    assert len(probes) == 1, f"{job_name} must run exactly one wheel probe"
    return probes[0]


def _probe_environment(job_name: str) -> dict[str, object]:
    """Return the probe step's environment mapping."""
    return mapping(_probe_step(job_name).get("env"), subject="probe environment")


def _built_interpreters() -> list[str]:
    """Return the sorted CPython versions the wheel builds target."""
    versions: list[str] = []
    for job_name in WHEEL_JOBS:
        for step in _steps(job_name):
            if not _is_wheel_build(step):
                continue
            inputs = mapping(step.get("with"), subject="maturin inputs")
            match = INTERPRETERS.search(str(inputs.get("args", "")))
            assert match is not None, f"{job_name} builds without interpreters"
            versions.extend(match.group(1).split())
    return sorted(set(versions))


@pytest.mark.parametrize("job_name", WHEEL_JOBS)
def test_each_wheel_build_probes_before_it_uploads(job_name: str) -> None:
    """A failed probe stops the upload instead of publishing a broken wheel."""
    steps = _steps(job_name)
    probe_index = steps.index(_probe_step(job_name))
    builds = [index for index, step in enumerate(steps) if _is_wheel_build(step)]
    uploads = [index for index, step in enumerate(steps) if _is_wheel_upload(step)]
    assert builds, f"{job_name} must build wheels"
    assert len(uploads) == 1, f"{job_name} must upload its wheels once"
    assert max(builds) < probe_index < uploads[0], (
        f"{job_name} must probe after its last build and before its upload"
    )


@pytest.mark.parametrize("job_name", WHEEL_JOBS)
def test_the_probes_cover_every_built_interpreter(job_name: str) -> None:
    """The probe set matches the wheel interpreter set, with nothing extra."""
    probed = str(_probe_environment(job_name).get("PROBE_VERSIONS", "")).split()
    assert probed == _built_interpreters(), (
        f"{job_name} must probe exactly the interpreters its wheels target"
    )


@pytest.mark.parametrize("script", PROBE_SCRIPTS, ids=lambda path: path.name)
def test_each_probe_script_defaults_to_the_probed_interpreters(script: Path) -> None:
    """A local run without ``PROBE_VERSIONS`` probes the same versions as CI."""
    defaults = re.search(r"\$\{PROBE_VERSIONS:-([^}]*)\}", script.read_text("utf-8"))
    assert defaults is not None, f"{script.name} must default PROBE_VERSIONS"
    assert defaults.group(1).split() == _built_interpreters(), (
        f"{script.name} defaults must match the interpreter set the wheels target"
    )


@pytest.mark.parametrize("script", PROBE_SCRIPTS, ids=lambda path: path.name)
def test_a_probe_runs_isolated_from_an_empty_directory(script: Path) -> None:
    """The checkout cannot satisfy the import the probe is meant to prove."""
    text = script.read_text("utf-8")
    scratch = re.search(r"""cd\s+[^\n]*?(?:mktemp|scratch|probe_dir)""", text)
    isolated = re.search(r"""-I\s+["']?\$\{?probe_""", text)
    assert scratch is not None, f"{script.name} needs a scratch directory"
    assert isolated is not None, f"{script.name} must run the probe with -I"
    assert scratch.end() < isolated.start(), (
        f"{script.name} must probe from its scratch directory, not the checkout"
    )


@pytest.mark.parametrize("script", PROBE_SCRIPTS, ids=lambda path: path.name)
def test_a_probe_refuses_a_missing_or_ambiguous_wheel(script: Path) -> None:
    """Probing nothing must fail, so an unbuilt interpreter cannot slip past."""
    text = script.read_text("utf-8")
    assert "expected exactly one CPython" in text, (
        f"{script.name} must name the cardinality it expects"
    )
    assert '"${#wheels[@]}" -ne 1' in text, (
        f"{script.name} must fail unless exactly one wheel matches"
    )


def _workflow_environment() -> dict[str, object]:
    """Return the workflow-level environment mapping."""
    document = yaml.safe_load((ROOT / WORKFLOW).read_text("utf-8"))
    return mapping(document.get("env"), subject="workflow environment")


def test_the_linux_probe_runs_in_the_manylinux_image_under_emulation() -> None:
    """Each Linux leg probes in its own manylinux image, aarch64 under QEMU."""
    steps = _steps("build-linux")
    environment = _probe_environment("build-linux")
    image = str(environment.get("PROBE_IMAGE", ""))
    pinned = ("MANYLINUX_X86_64_PROBE", "MANYLINUX_AARCH64_PROBE")
    assert all(name in image for name in pinned), (
        "the probe selects its image from the digest-pinned workflow constants"
    )
    workflow_environment = _workflow_environment()
    for name in pinned:
        reference = str(workflow_environment.get(name, ""))
        assert reference.startswith("quay.io/pypa/manylinux_2_28_"), (
            f"{name} must name a manylinux 2_28 image"
        )
        assert re.fullmatch(r".+@sha256:[0-9a-f]{64}", reference), (
            f"{name} must pin its image by digest"
        )
        arch = name.removeprefix("MANYLINUX_").removesuffix("_PROBE").lower()
        assert arch in reference, f"{name} must name the {arch} image"
    platform = str(environment.get("PROBE_PLATFORM", ""))
    assert "linux/amd64" in platform, "the x86_64 probe runs natively"
    assert "linux/arm64" in platform, "the aarch64 probe runs under emulation"
    qemu = [
        index
        for index, step in enumerate(steps)
        if str(step.get("uses", "")).startswith("docker/setup-qemu-action@")
    ]
    assert qemu, "the aarch64 leg must set up QEMU before probing"
    assert qemu[0] < steps.index(_probe_step("build-linux")), (
        "QEMU emulation must be ready before the aarch64 probe runs"
    )
