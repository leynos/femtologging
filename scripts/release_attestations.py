"""Match Sigstore attestation bundles to the release bytes they cover.

The release workflow attests the files it builds and attaches that run's
provenance bundle to the GitHub Release. A partial-release rerun can carry
artefacts over from PyPI instead of uploading its fresh build, so the fresh
bundle may attest bytes the release never publishes. This module reads each
bundle's in-toto subjects and decides which bundles may be attached, and
whether every published artefact is covered by an attestation.
"""

from __future__ import annotations

import base64
import json
import typing as typ

from scripts.release_asset_errors import MalformedAttestationError

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

__all__ = [
    "bundle_subjects",
    "partition_bundles",
    "select_bundles",
    "uncovered_artefacts",
    "uncovered_names",
]


def bundle_subjects(bundle: Path) -> dict[str, str]:
    """Return the in-toto subject names and SHA-256 digests in a bundle.

    Parameters
    ----------
    bundle
        Sigstore bundle written by ``actions/attest-build-provenance``.

    Returns
    -------
    dict[str, str]
        Subject filenames mapped to lowercase SHA-256 digests.

    Raises
    ------
    MalformedAttestationError
        If the bundle has no readable DSSE payload or no subjects.

    Examples
    --------
    >>> import base64, json, tempfile
    >>> from pathlib import Path
    >>> statement = {"subject": [{"name": "a.whl", "digest": {"sha256": "AB"}}]}
    >>> payload = base64.b64encode(json.dumps(statement).encode()).decode()
    >>> bundle = Path(tempfile.mkdtemp()) / "run.sigstore.json"
    >>> _ = bundle.write_text(json.dumps({"dsseEnvelope": {"payload": payload}}))
    >>> bundle_subjects(bundle)
    {'a.whl': 'ab'}
    """
    try:
        document = json.loads(bundle.read_text(encoding="utf-8"))
        payload = base64.b64decode(document["dsseEnvelope"]["payload"], validate=True)
        statement = json.loads(payload)
        subjects = {
            str(subject["name"]): str(subject["digest"]["sha256"]).lower()
            for subject in statement["subject"]
        }
    except (KeyError, TypeError, ValueError) as error:
        raise MalformedAttestationError(bundle.name) from error
    if not subjects:
        raise MalformedAttestationError(bundle.name)
    return subjects


def partition_bundles(
    subjects_by_bundle: cabc.Mapping[str, cabc.Mapping[str, str]],
    release_digests: cabc.Mapping[str, str],
) -> tuple[list[str], list[str]]:
    """Split decoded bundle subjects into attachable and rejected names.

    This is the network- and filesystem-free decision behind
    :func:`select_bundles`. A bundle is attachable only when every subject
    digest it attests is the digest of an artefact the release publishes.

    Parameters
    ----------
    subjects_by_bundle
        Bundle filenames mapped to the in-toto subjects they attest.
    release_digests
        Published artefact names mapped to their canonical SHA-256 digests.

    Returns
    -------
    tuple[list[str], list[str]]
        Attachable bundle names and rejected bundle names, both sorted.

    Examples
    --------
    >>> partition_bundles(
    ...     {"a.json": {"a.whl": "aa"}, "b.json": {"b.whl": "bb"}},
    ...     {"a.whl": "aa"},
    ... )
    (['a.json'], ['b.json'])
    """
    published = set(release_digests.values())
    attachable: list[str] = []
    rejected: list[str] = []
    for name in sorted(subjects_by_bundle):
        if set(subjects_by_bundle[name].values()) <= published:
            attachable.append(name)
        else:
            rejected.append(name)
    return attachable, rejected


def uncovered_names(
    subjects_by_bundle: cabc.Mapping[str, cabc.Mapping[str, str]],
    release_digests: cabc.Mapping[str, str],
) -> list[str]:
    """List published artefacts no decoded bundle attests with their digest.

    This is the network- and filesystem-free decision behind
    :func:`uncovered_artefacts`.

    Parameters
    ----------
    subjects_by_bundle
        Attached bundle filenames mapped to the in-toto subjects they attest.
    release_digests
        Published artefact names mapped to their canonical SHA-256 digests.

    Returns
    -------
    list[str]
        Sorted artefact names whose digest appears in no bundle.

    Examples
    --------
    >>> uncovered_names({"a.json": {"a.whl": "aa"}}, {"a.whl": "aa", "b": "bb"})
    ['b']
    """
    covered = {
        digest
        for subjects in subjects_by_bundle.values()
        for digest in subjects.values()
    }
    return [
        name
        for name, digest in sorted(release_digests.items())
        if digest not in covered
    ]


def _decode_bundles(bundles: cabc.Iterable[Path]) -> dict[str, dict[str, str]]:
    """Read every bundle's in-toto subjects, keyed by bundle filename."""
    return {bundle.name: bundle_subjects(bundle) for bundle in sorted(bundles)}


def select_bundles(
    bundles: cabc.Iterable[Path],
    release_digests: cabc.Mapping[str, str],
) -> tuple[list[Path], list[str]]:
    """Split bundles into attachable ones and rejected names.

    A bundle is attachable only when every subject digest it attests is the
    digest of an artefact the release publishes. Attaching a bundle that
    attests other bytes would misrepresent the release, so it is rejected
    instead.

    Parameters
    ----------
    bundles
        Candidate bundles, usually this run's attestation directory.
    release_digests
        Published artefact names mapped to their canonical SHA-256 digests.

    Returns
    -------
    tuple[list[Path], list[str]]
        Attachable bundles in sorted order, and the names of rejected ones.

    Examples
    --------
    >>> import base64, json, tempfile
    >>> from pathlib import Path
    >>> def bundle(name, digest):
    ...     statement = {"subject": [{"name": name, "digest": {"sha256": digest}}]}
    ...     payload = base64.b64encode(json.dumps(statement).encode()).decode()
    ...     path = Path(tempfile.mkdtemp()) / f"{name}.sigstore.json"
    ...     _ = path.write_text(json.dumps({"dsseEnvelope": {"payload": payload}}))
    ...     return path
    >>> good, bad = bundle("a.whl", "aa"), bundle("b.whl", "bb")
    >>> attachable, rejected = select_bundles([good, bad], {"a.whl": "aa"})
    >>> [path.name for path in attachable], rejected
    (['a.whl.sigstore.json'], ['b.whl.sigstore.json'])
    """
    paths = {bundle.name: bundle for bundle in bundles}
    attachable, rejected = partition_bundles(
        _decode_bundles(paths.values()), release_digests
    )
    return [paths[name] for name in attachable], rejected


def uncovered_artefacts(
    bundles: cabc.Iterable[Path],
    release_digests: cabc.Mapping[str, str],
) -> list[str]:
    """List published artefacts no bundle attests with their digest.

    Parameters
    ----------
    bundles
        Bundles that are attached to the release.
    release_digests
        Published artefact names mapped to their canonical SHA-256 digests.

    Returns
    -------
    list[str]
        Sorted artefact names whose digest appears in no bundle.

    Examples
    --------
    >>> import base64, json, tempfile
    >>> from pathlib import Path
    >>> statement = {"subject": [{"name": "a.whl", "digest": {"sha256": "aa"}}]}
    >>> payload = base64.b64encode(json.dumps(statement).encode()).decode()
    >>> bundle = Path(tempfile.mkdtemp()) / "run.sigstore.json"
    >>> _ = bundle.write_text(json.dumps({"dsseEnvelope": {"payload": payload}}))
    >>> uncovered_artefacts([bundle], {"a.whl": "aa", "b.whl": "bb"})
    ['b.whl']
    """
    return uncovered_names(_decode_bundles(bundles), release_digests)
