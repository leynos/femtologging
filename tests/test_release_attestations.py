"""Unit contracts for matching provenance bundles to release bytes."""

from __future__ import annotations

import base64
import json
import typing as typ

import pytest

from scripts.release_asset_errors import MalformedAttestationError
from scripts.release_attestations import (
    bundle_subjects,
    select_bundles,
    uncovered_artefacts,
)

if typ.TYPE_CHECKING:
    from pathlib import Path


def _bundle(path: Path, subjects: dict[str, str]) -> Path:
    """Write a minimal Sigstore bundle attesting the given subjects."""
    statement = {
        "subject": [
            {"name": name, "digest": {"sha256": digest}}
            for name, digest in subjects.items()
        ]
    }
    payload = base64.b64encode(json.dumps(statement).encode()).decode()
    path.parent.mkdir(parents=True, exist_ok=True)
    document = {"dsseEnvelope": {"payload": payload}}
    path.write_text(json.dumps(document), encoding="utf-8")
    return path


def test_bundle_subjects_lowercases_digests(tmp_path: Path) -> None:
    """Subject names and digests are read from the DSSE payload."""
    bundle = _bundle(tmp_path / "run.sigstore.json", {"a.whl": "AB", "b.tar.gz": "CD"})

    assert bundle_subjects(bundle) == {"a.whl": "ab", "b.tar.gz": "cd"}, (
        "Subject names and digests are read from the DSSE payload."
    )


@pytest.mark.parametrize(
    "document",
    [
        pytest.param({}, id="no-dsse-envelope"),
        pytest.param({"dsseEnvelope": {"payload": "not base64!"}}, id="bad-payload"),
        pytest.param(
            {"dsseEnvelope": {"payload": base64.b64encode(b"{}").decode()}},
            id="no-subjects",
        ),
        pytest.param(
            {
                "dsseEnvelope": {
                    "payload": base64.b64encode(b'{"subject": []}').decode()
                }
            },
            id="empty-subjects",
        ),
    ],
)
def test_malformed_bundles_are_rejected(
    tmp_path: Path, document: dict[str, typ.Any]
) -> None:
    """A bundle without readable subjects never counts as an attestation."""
    bundle = tmp_path / "broken.sigstore.json"
    bundle.write_text(json.dumps(document), encoding="utf-8")

    with pytest.raises(MalformedAttestationError, match=r"broken\.sigstore\.json"):
        bundle_subjects(bundle)


def test_select_bundles_keeps_only_bundles_for_published_bytes(
    tmp_path: Path,
) -> None:
    """A bundle attesting any unpublished digest is rejected as a whole."""
    matching = _bundle(tmp_path / "matching" / "a.sigstore.json", {"a.whl": "aa"})
    stale = _bundle(
        tmp_path / "stale" / "b.sigstore.json",
        {"a.whl": "aa", "sdist.tar.gz": "bb"},
    )

    attachable, rejected = select_bundles(
        [matching, stale], {"a.whl": "aa", "sdist.tar.gz": "cc"}
    )

    assert [path.name for path in attachable] == ["a.sigstore.json"], (
        "Only a bundle whose every subject matches published bytes is attachable."
    )
    assert rejected == ["b.sigstore.json"], (
        "A bundle attesting unpublished bytes is rejected by name."
    )


def test_uncovered_artefacts_names_every_unattested_artefact(
    tmp_path: Path,
) -> None:
    """Artefacts whose digest appears in no bundle are reported in order."""
    bundles = [
        _bundle(tmp_path / "run-1" / "a.sigstore.json", {"a.whl": "aa"}),
        _bundle(tmp_path / "run-2" / "b.sigstore.json", {"b.whl": "bb"}),
    ]

    assert uncovered_artefacts(
        bundles, {"a.whl": "aa", "b.whl": "bb", "c.tar.gz": "cc"}
    ) == ["c.tar.gz"], "Only the artefact no bundle attests is reported."


def test_uncovered_artefacts_compares_digests_not_names(tmp_path: Path) -> None:
    """An attestation for the same name with other bytes does not cover it."""
    bundles = [_bundle(tmp_path / "run" / "a.sigstore.json", {"a.whl": "stale"})]

    assert uncovered_artefacts(bundles, {"a.whl": "published"}) == ["a.whl"], (
        "A subject with the right name but other bytes leaves the artefact bare."
    )
