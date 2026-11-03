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

__all__ = ["bundle_subjects", "select_bundles", "uncovered_artefacts"]


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
    published = set(release_digests.values())
    attachable: list[Path] = []
    rejected: list[str] = []
    for bundle in sorted(bundles):
        if set(bundle_subjects(bundle).values()) <= published:
            attachable.append(bundle)
        else:
            rejected.append(bundle.name)
    return attachable, rejected


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
    covered: set[str] = set()
    for bundle in bundles:
        covered.update(bundle_subjects(bundle).values())
    return [
        name
        for name, digest in sorted(release_digests.items())
        if digest not in covered
    ]
