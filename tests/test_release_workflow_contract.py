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


def test_workflows_do_not_override_reserved_runner_variables() -> None:
    """Custom environment values do not shadow runner-owned variables."""
    reserved = {"GITHUB_REPOSITORY", "RUNNER_OS", "RUNNER_TEMP"}
    for workflow in (RELEASE_WORKFLOW, BUILD_WORKFLOW):
        document = _workflow(workflow)
        jobs = mapping(document.get("jobs"), subject=f"{workflow} jobs")
        environments = []
        if "env" in document:
            environments.append(mapping(document["env"], subject="workflow env"))
        for job_name, job in jobs.items():
            job_document = mapping(job, subject=job_name)
            if "env" in job_document:
                environments.append(
                    mapping(job_document["env"], subject=f"{job_name} env")
                )
            environments.extend(
                mapping(step["env"], subject="step env")
                for step in objects(job_document.get("steps", []), subject="steps")
                if "env" in step
            )
        assert all(not reserved.intersection(env) for env in environments), (
            f"{workflow} must not overwrite GitHub-owned environment variables."
        )


def test_linux_wheel_builds_prepare_openssl_perl_and_target_compilers() -> None:
    """Linux wheel containers provide Perl modules and target C compilers."""
    x86_build = sole_workflow_step(
        BUILD_WORKFLOW, "build-linux", "Build Linux x86_64 wheels"
    )
    x86_script = mapping(x86_build.get("with"), subject="x86_64 maturin inputs").get(
        "before-script-linux"
    )
    assert isinstance(x86_script, str), "The x86_64 build uses a shell script."
    assert (
        re.search(
            r"yum -y install perl-IPC-Cmd perl-Time-Piece\s+fi\s+"
            r"perl -MIPC::Cmd -MFindBin -MTime::Piece",
            x86_script,
        )
        is not None
    ), "The minimal manylinux image installs OpenSSL's missing Perl modules."

    arm_build = sole_workflow_step(
        BUILD_WORKFLOW, "build-linux", "Build Linux aarch64 wheels"
    )
    arm_script = mapping(arm_build.get("with"), subject="aarch64 maturin inputs").get(
        "before-script-linux"
    )
    assert isinstance(arm_script, str), "The aarch64 build uses a shell script."
    assert "perl -MIPC::Cmd -MFindBin -MTime::Piece" in arm_script, (
        "The aarch64 build checks OpenSSL's required Perl modules."
    )
    assert (
        re.search(
            r"command -v aarch64-unknown-linux-gnu-gcc\s+case.*?"
            r"aarch64-unknown-linux-gnu\*",
            arm_script,
            re.DOTALL,
        )
        is not None
    ), "The aarch64 container checks its target C compiler."

    for job_name in ("build-other", "verify-wheel-install"):
        step = sole_workflow_step(
            BUILD_WORKFLOW, job_name, "Verify vendored OpenSSL build tools"
        )
        script = step.get("run")
        assert isinstance(script, str), f"{job_name} checks OpenSSL build tools."
        assert "perl -MIPC::Cmd -MFindBin -MTime::Piece" in script, (
            f"{job_name} checks OpenSSL's required Perl modules."
        )


def test_windows_wheel_build_uses_the_complete_runner_perl() -> None:
    """The Windows wheel build prioritizes the runner's full Perl install."""
    step = sole_workflow_step(
        BUILD_WORKFLOW, "build-other", "Verify vendored OpenSSL build tools"
    )
    script = step.get("run")
    assert isinstance(script, str), "The Windows toolchain check has a shell script."
    assert (
        re.search(
            r"echo 'C:.Strawberry.perl.site.bin' >> \"\$\{GITHUB_PATH\}\"\s+"
            r"echo 'C:.Strawberry.perl.bin' >> \"\$\{GITHUB_PATH\}\"\s+"
            r"export PATH=\"/c/Strawberry/perl/site/bin:"
            r"/c/Strawberry/perl/bin:\$\{PATH\}\"",
            script,
        )
        is not None
    ), "The Windows wheel build prioritizes the runner's full Perl install."


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
    for job_name in (
        "build",
        "attest",
        "draft-release",
        "publish-pypi",
        "publish-release",
    ):
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
    refresh = sole_workflow_step(
        RELEASE_WORKFLOW,
        "publish-pypi",
        "Fetch the PyPI index for GitHub reconciliation",
    )
    assert (
        mapping(refresh.get("env"), subject="PyPI index refresh environment").get(
            "PYPI_EXPECTED_DIRECTORY"
        )
        == "dist/publish"
    ), "The attested files are handed forward and existing PyPI names are safe."
    assert (
        mapping(refresh.get("env"), subject="PyPI index refresh environment").get(
            "PYPI_ALLOW_MISSING"
        )
        == "true"
    ), "The attested files are handed forward and existing PyPI names are safe."
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


def test_canonical_pypi_download_restricts_redirects_to_https() -> None:
    """The canonical-byte fetch rejects insecure initial and redirected URLs."""
    fetch = sole_workflow_step(
        RELEASE_WORKFLOW,
        "publish-release",
        "Fetch canonical PyPI bytes and refresh its index",
    )
    script = fetch.get("run")
    assert isinstance(script, str), "The canonical PyPI fetch step has a shell script."
    assert "--proto '=https' --proto-redir '=https'" in script, (
        "The canonical PyPI fetch step permits only HTTPS requests and redirects."
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
