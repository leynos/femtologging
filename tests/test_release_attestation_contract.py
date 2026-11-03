"""Structural contracts for binding release attestations to published bytes."""

from __future__ import annotations

from tests.make_contract_helpers import sole_workflow_step

RELEASE_WORKFLOW = ".github/workflows/release.yml"


def test_verification_uses_bundles_already_attached_to_the_release() -> None:
    """Verification can cover carried-over bytes with an earlier attestation."""
    download = sole_workflow_step(
        RELEASE_WORKFLOW, "publish-release", "Download existing attestation bundles"
    )
    download_script = str(download.get("run"))
    assert 'endswith(".sigstore.json")' in download_script, (
        "Only attestation bundles are downloaded from the release."
    )
    verify = sole_workflow_step(
        RELEASE_WORKFLOW, "publish-release", "Verify PyPI and GitHub Release digests"
    )
    verify_script = str(verify.get("run"))
    assert '--existing-bundles "${EXISTING_BUNDLES}"' in verify_script, (
        "Verification receives the bundles an earlier run attached."
    )
