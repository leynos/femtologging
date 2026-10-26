"""Read and write the filesystem records used by release reconciliation.

This module is standard-library-only. It validates package filenames and
digests while the workflow keeps network requests in ``curl`` and ``gh`` steps.
"""

from __future__ import annotations

import dataclasses as dc
import hashlib
import json
import os
import re
import urllib.parse
from pathlib import Path

from scripts.release_asset_errors import (
    MalformedDigestError,
    MissingDigestError,
    ReleaseAssetError,
    UnsafeArtifactNamesError,
)

#: The PEP 691 JSON Simple index; relative file URLs resolve against it.
INDEX_URL = "https://pypi.org/simple/femtologging/"
_SAFE_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._+-]*")
_SHA256 = re.compile(r"[0-9a-f]{64}")
_DIGEST_PREFIX = "sha256:"


@dc.dataclass(frozen=True, slots=True)
class PypiFile:
    """One file listed by the PyPI Simple index.

    Attributes
    ----------
    sha256
        Lowercase SHA-256 digest for the file.
    url
        Absolute HTTPS URL from which the file can be downloaded.
    """

    sha256: str
    url: str


def _validated_sha256(value: object, source: str) -> str:
    """Return a lowercase SHA-256 value or reject its malformed spelling."""
    if not isinstance(value, str):
        raise MissingDigestError(source)
    digest = value.lower()
    if _SHA256.fullmatch(digest) is None:
        raise MalformedDigestError(source)
    return digest


def read_pypi_index(path: Path) -> dict[str, PypiFile]:
    """Read PyPI filenames, normalized hashes and download URLs.

    Parameters
    ----------
    path
        Path to a PEP 691 JSON Simple index response.

    Returns
    -------
    dict[str, PypiFile]
        Records keyed by the exact filename published by PyPI.

    Examples
    --------
    Reading an index maps each filename to its digest and resolved URL.
    """
    document = json.loads(path.read_text(encoding="utf-8"))
    files: dict[str, PypiFile] = {}
    for entry in document.get("files", []):
        name = entry["filename"]
        digest = _validated_sha256(entry["hashes"].get("sha256"), name)
        files[name] = PypiFile(
            sha256=digest,
            url=urllib.parse.urljoin(INDEX_URL, entry["url"]),
        )
    return files


def _digest_hex(value: object) -> str | None:
    """Read only a well-formed GitHub ``sha256:`` digest."""
    if not isinstance(value, str) or not value.startswith(_DIGEST_PREFIX):
        return None
    try:
        return _validated_sha256(value.removeprefix(_DIGEST_PREFIX), "GitHub")
    except ReleaseAssetError:
        return None


def read_github_assets(path: Path) -> dict[str, str | None]:
    """Read GitHub Release asset filenames and their SHA-256 digests.

    Parameters
    ----------
    path
        Path to the JSON object returned by ``gh release view --json assets``.

    Returns
    -------
    dict[str, str | None]
        Asset names mapped to lowercase digests. A missing or malformed digest
        is represented by ``None`` so verification fails safely later.

    Examples
    --------
    A valid ``sha256:`` asset digest is returned without its prefix.
    """
    document = json.loads(path.read_text(encoding="utf-8"))
    return {
        asset["name"]: _digest_hex(asset.get("digest"))
        for asset in document.get("assets", [])
    }


def sha256_of(path: Path) -> str:
    """Return the lowercase SHA-256 digest of a file's bytes.

    Parameters
    ----------
    path
        File to hash.

    Returns
    -------
    str
        Lowercase hexadecimal SHA-256 digest.

    Examples
    --------
    Hashing a wheel returns the digest used to compare its published copies.
    """
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def local_names(directory: Path) -> list[str]:
    """List safe artefact filenames in a staging directory.

    Parameters
    ----------
    directory
        Directory containing the local wheels, source archive or bundle.

    Returns
    -------
    list[str]
        Sorted filenames for regular files in ``directory``.

    Raises
    ------
    UnsafeArtifactNamesError
        If a filename could be unsafe to pass between workflow steps.

    Examples
    --------
    A directory containing ``package.whl`` returns ``["package.whl"]``.
    """
    names = sorted(path.name for path in directory.iterdir() if path.is_file())
    unsafe = [name for name in names if _SAFE_NAME.fullmatch(name) is None]
    if unsafe:
        raise UnsafeArtifactNamesError(unsafe)
    return names


def write_output(name: str, *, value: bool) -> None:
    """Append a lowercase Boolean value to the workflow output file.

    Parameters
    ----------
    name
        Output name consumed by a later GitHub Actions step.
    value
        Boolean value encoded as ``true`` or ``false``.

    Examples
    --------
    ``write_output("remaining", value=False)`` writes ``remaining=false``.
    """
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        output.write(f"{name}={str(value).lower()}\n")
