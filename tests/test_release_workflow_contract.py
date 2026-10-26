"""Structural contracts for wheel and release workflow hardening."""

from __future__ import annotations

import re
import typing as typ

import yaml

from tests.make_contract_helpers import (
    mapping,
    objects,
    repo_root,
    sole_workflow_step,
    text_sequence,
    workflow_job,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

RELEASE_WORKFLOW = ".github/workflows/release.yml"
BUILD_WORKFLOW = ".github/workflows/build-wheels.yml"
CI_WORKFLOW = ".github/workflows/ci.yml"
_FULL_SHA = re.compile(r"@[0-9a-f]{40}$")


def _workflow(path: str) -> dict[str, object]:
    """Read one workflow as a mapping using the repository root."""
    return mapping(
        yaml.safe_load((repo_root() / path).read_text(encoding="utf-8")),
        subject=path,
    )


def _release_steps(job_name: str) -> list[dict[str, object]]:
    """Return the parsed steps for a release job."""
    return objects(
        workflow_job(RELEASE_WORKFLOW, job_name).get("steps"),
        subject=f"release.yml {job_name} steps",
    )


def _run_scripts(document: object, where: str) -> cabc.Iterator[tuple[str, str]]:
    """Yield every shell script recursively with its workflow location."""
    match document:
        case dict() as entries:
            run = entries.get("run")
            if isinstance(run, str):
                yield where, run
            for key, value in entries.items():
                yield from _run_scripts(value, f"{where}.{key}")
        case list() as items:
            for index, value in enumerate(items):
                yield from _run_scripts(value, f"{where}[{index}]")


def test_release_jobs_have_only_the_required_permissions() -> None:
    """The release workflow starts with no scope and grants exact job rights."""
    release = _workflow(RELEASE_WORKFLOW)
    assert release.get("permissions") == {}, (
        "The release workflow starts with no scope and grants exact job rights."
    )
    expected = {
        "check-version": {"contents": "read"},
        "build": {"contents": "read"},
        "attest": {
            "contents": "read",
            "id-token": "write",
            "attestations": "write",
        },
        "draft-release": {"contents": "write"},
        "publish-pypi": {"contents": "read", "id-token": "write"},
        "publish-release": {"contents": "write"},
    }
    jobs = mapping(release.get("jobs"), subject="release jobs")
    assert {
        name: mapping(job, subject=name).get("permissions")
        for name, job in jobs.items()
    } == expected, (
        "The release workflow starts with no scope and grants exact job rights."
    )


def test_pypi_environment_and_release_concurrency_are_protected() -> None:
    """Trusted Publishing uses `pypi`, and repeated tags queue safely."""
    assert workflow_job(RELEASE_WORKFLOW, "publish-pypi").get("environment") == {
        "name": "pypi",
        "url": "https://pypi.org/project/femtologging/",
    }, "Trusted Publishing uses `pypi`, and repeated tags queue safely."
    assert _workflow(RELEASE_WORKFLOW).get("concurrency") == {
        "group": "release-${{ github.ref }}",
        "cancel-in-progress": False,
    }, "Trusted Publishing uses `pypi`, and repeated tags queue safely."


def test_every_checkout_disables_persisted_credentials() -> None:
    """Release and build checkouts never keep their repository token on disk."""
    for workflow in (RELEASE_WORKFLOW, BUILD_WORKFLOW):
        steps: list[dict[str, object]] = []
        for job in mapping(_workflow(workflow).get("jobs"), subject="jobs").values():
            job_steps = mapping(job, subject="job").get("steps")
            if job_steps is not None:
                steps.extend(
                    step
                    for step in objects(job_steps, subject="steps")
                    if str(step.get("uses", "")).startswith("actions/checkout@")
                )
        assert steps, f"{workflow} must contain a checkout"
        for step in steps:
            assert _FULL_SHA.search(str(step["uses"])), "checkout must use a full SHA"
            assert (
                mapping(step.get("with"), subject="checkout inputs").get(
                    "persist-credentials"
                )
                is False
            ), "Release and build checkouts never keep their repository token on disk."


def test_version_checks_gate_every_publishing_job() -> None:
    """Both manifests match the same tag before assets can be published."""
    cargo = sole_workflow_step(
        RELEASE_WORKFLOW,
        "check-version",
        "Check the Rust extension version against the tag",
    )
    assert cargo.get("uses") == (
        "leynos/shared-actions/.github/actions/ensure-cargo-version@"
        "2f58fab140cc8dfe69756aaa6a3bbb987c426023"
    ), "Both manifests match the same tag before assets can be published."
    assert cargo.get("with") == {"manifests": "rust_extension/Cargo.toml"}, (
        "Both manifests match the same tag before assets can be published."
    )
    version = sole_workflow_step(
        RELEASE_WORKFLOW,
        "check-version",
        "Check the Python project version against the tag",
    )
    assert (
        mapping(version.get("env"), subject="version environment").get("TAG_NAME")
        == "${{ github.ref_name }}"
    ), "Both manifests match the same tag before assets can be published."

    jobs = mapping(_workflow(RELEASE_WORKFLOW).get("jobs"), subject="release jobs")
    for job_name in ("attest", "draft-release", "publish-pypi", "publish-release"):
        needs = mapping(jobs.get(job_name), subject=job_name).get("needs")
        parents = (
            {needs}
            if isinstance(needs, str)
            else set(text_sequence(needs or [], subject=f"{job_name} needs"))
        )
        assert "check-version" in parents, f"{job_name} must wait for check-version"
    release_needs = text_sequence(
        mapping(jobs.get("publish-release"), subject="publish-release").get("needs"),
        subject="publish-release needs",
    )
    assert set(release_needs) == {
        "check-version",
        "attest",
        "publish-pypi",
        "draft-release",
    }, "Both manifests match the same tag before assets can be published."


def test_attestation_and_idempotent_pypi_upload_use_the_required_actions() -> None:
    """The attested files are handed forward and existing PyPI names are safe."""
    attest = next(
        step
        for step in _release_steps("attest")
        if str(step.get("uses", "")).startswith("actions/attest-build-provenance@")
    )
    assert _FULL_SHA.search(str(attest["uses"])), (
        "The attested files are handed forward and existing PyPI names are safe."
    )
    assert attest.get("with") == {"subject-path": "dist/publish/*"}, (
        "The attested files are handed forward and existing PyPI names are safe."
    )
    upload = next(
        step
        for step in _release_steps("publish-pypi")
        if str(step.get("uses", "")).startswith("pypa/gh-action-pypi-publish@")
    )
    assert _FULL_SHA.search(str(upload["uses"])), (
        "The attested files are handed forward and existing PyPI names are safe."
    )
    assert upload.get("if") == "steps.stage-pypi.outputs.remaining == 'true'", (
        "The attested files are handed forward and existing PyPI names are safe."
    )
    assert upload.get("with") == {
        "packages-dir": "dist/publish/",
        "skip-existing": True,
    }, "The attested files are handed forward and existing PyPI names are safe."
    for job_name in ("publish-pypi", "draft-release", "publish-release"):
        artifacts = [
            step.get("with")
            for step in _release_steps(job_name)
            if str(step.get("uses", "")).startswith("actions/download-artifact@")
        ]
        assert {"name": "release-dist", "path": "dist"} in artifacts, (
            "The attested files are handed forward and existing PyPI names are safe."
        )


def test_release_asset_reconciliation_precedes_publication() -> None:
    """Carry-over, staging, digest verification, and publication stay ordered."""
    steps = _release_steps("publish-release")
    names = [step.get("name") for step in steps]
    required_order = [
        "Snapshot GitHub asset names and digests",
        "Carry over existing GitHub assets",
        "Stage missing GitHub assets",
        "Fetch canonical PyPI bytes and refresh its index",
        "Check all fetched PyPI bytes",
        "Upload missing assets to the GitHub Release",
        "Verify PyPI and GitHub Release digests",
        "Publish the GitHub Release",
    ]
    positions = [names.index(name) for name in required_order]
    assert positions == sorted(positions), (
        "Carry-over, staging, digest verification, and publication stay ordered."
    )
    publish = sole_workflow_step(
        RELEASE_WORKFLOW, "publish-release", "Publish the GitHub Release"
    )
    assert "--draft=false" in str(publish.get("run")), (
        "Carry-over, staging, digest verification, and publication stay ordered."
    )
    assert '--prerelease="${IS_PRERELEASE}"' in str(publish.get("run")), (
        "Carry-over, staging, digest verification, and publication stay ordered."
    )


def test_both_build_and_release_workflows_harden_shell_scripts() -> None:
    """Run values enter through env and GitHub assets are never clobbered."""
    for path in (BUILD_WORKFLOW, RELEASE_WORKFLOW):
        scripts = list(_run_scripts(_workflow(path), path))
        assert scripts, f"{path} must have shell steps"
        for location, script in scripts:
            assert "${{" not in script, f"template expansion in shell at {location}"
            assert "set -euo pipefail" in script, (
                f"strict shell mode missing at {location}"
            )
            assert "--clobber" not in script, f"asset overwrite at {location}"


def test_pull_request_ci_calls_the_reusable_sdist_build() -> None:
    """Every pull request gets the same native wheel and one-sdist checks."""
    caller = workflow_job(CI_WORKFLOW, "build-wheels")
    assert caller.get("uses") == "./.github/workflows/build-wheels.yml", (
        "Every pull request gets the same native wheel and one-sdist checks."
    )
    assert caller.get("permissions") == {"contents": "read"}, (
        "Every pull request gets the same native wheel and one-sdist checks."
    )
    assert _workflow(BUILD_WORKFLOW).get("permissions") == {}, (
        "Every pull request gets the same native wheel and one-sdist checks."
    )
    cardinality = sole_workflow_step(
        BUILD_WORKFLOW,
        "verify-wheel-install",
        "Check source distribution cardinality",
    )
    run = str(cardinality.get("run"))
    assert "Expected exactly one sdist, found none." in run, (
        "Every pull request gets the same native wheel and one-sdist checks."
    )
    assert "Expected exactly one sdist, found multiple." in run, (
        "Every pull request gets the same native wheel and one-sdist checks."
    )
