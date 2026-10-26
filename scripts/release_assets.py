"""Reconcile release artefacts between PyPI and the GitHub Release.

For each filename, PyPI's bytes are canonical first, followed by an existing
GitHub Release asset, and then this run's build. Each destination receives
only missing names. This module has no network access; the workflow performs
all requests through ``curl`` and ``gh``.
"""

from __future__ import annotations

import dataclasses as dc
import enum
import hashlib
import json
import os
import re
import shutil
import typing as typ
import urllib.parse
from pathlib import Path

from scripts.release_asset_errors import (
    ArtifactDigestMismatchError,
    DestinationDigestMismatchError,
    MalformedDigestError,
    MissingDigestError,
    NonHttpsDownloadError,
    ReleaseAssetError,
    UnsafeArtifactNamesError,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

#: The PEP 691 JSON Simple index; relative file URLs resolve against it.
INDEX_URL = "https://pypi.org/simple/femtologging/"
_SAFE_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._+-]*")
_SHA256 = re.compile(r"[0-9a-f]{64}")
_DIGEST_PREFIX = "sha256:"


class Source(enum.StrEnum):
    """Where one artefact name's canonical bytes come from."""

    PYPI = "pypi"
    GITHUB = "github"
    LOCAL = "local"


@dc.dataclass(frozen=True, slots=True)
class Plan:
    """The canonical source of each name and what each destination lacks."""

    canonical: cabc.Mapping[str, Source]
    pypi_uploads: frozenset[str]
    github_uploads: frozenset[str]


def plan(
    local: cabc.Iterable[str],
    on_pypi: cabc.Collection[str],
    on_github: cabc.Collection[str],
) -> Plan:
    """Choose canonical bytes and the filenames each destination needs.

    Parameters
    ----------
    local
        Artefact names built in this run.
    on_pypi
        Names already held by PyPI.
    on_github
        Names already held by the tag's GitHub Release.

    Returns
    -------
    Plan
        PyPI precedes GitHub, which precedes this run's build.

    Examples
    --------
    >>> plan(["a.whl"], set(), {"a.whl"}).canonical["a.whl"]
    <Source.GITHUB: 'github'>
    """
    names = frozenset(local)
    canonical = {
        name: Source.PYPI
        if name in on_pypi
        else Source.GITHUB
        if name in on_github
        else Source.LOCAL
        for name in names
    }
    return Plan(
        canonical=canonical,
        pypi_uploads=frozenset(name for name in names if name not in on_pypi),
        github_uploads=frozenset(name for name in names if name not in on_github),
    )


def mismatches(
    local: cabc.Iterable[str],
    pypi_digests: cabc.Mapping[str, str],
    github_digests: cabc.Mapping[str, str | None],
) -> list[str]:
    """Describe files PyPI and the GitHub Release do not hold identically.

    Parameters
    ----------
    local
        Artefact names that must exist at both destinations.
    pypi_digests
        SHA-256 values reported by PyPI for each filename.
    github_digests
        GitHub SHA-256 values, or ``None`` when GitHub reports no valid digest.

    Returns
    -------
    list[str]
        A message for each missing filename or differing digest.

    Examples
    --------
    >>> mismatches(["a.whl"], {"a.whl": "ab"}, {"a.whl": "ab"})
    []
    """
    return [
        issue
        for name in sorted(set(local))
        if (issue := _mismatch(name, pypi_digests, github_digests))
    ]


def _mismatch(
    name: str,
    pypi_digests: cabc.Mapping[str, str],
    github_digests: cabc.Mapping[str, str | None],
) -> str | None:
    """Describe one missing or differing destination asset, if any."""
    pypi = pypi_digests.get(name)
    github = github_digests.get(name)
    if pypi is None:
        return f"{name} is missing from PyPI"
    if github is None:
        return f"{name} is missing from the GitHub Release or has no SHA-256 digest"
    if pypi != github:
        return f"{name} differs: PyPI sha256 {pypi}, GitHub sha256 {github}"
    return None


@dc.dataclass(frozen=True, slots=True)
class PypiFile:
    """One file listed by the PyPI Simple index."""

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
    """Map PyPI filenames to lowercase SHA-256 hashes and HTTPS URLs.

    Returns
    -------
    dict[str, PypiFile]
        The filenames, normalized hashes, and index-relative download URLs.

    Examples
    --------
    A valid PEP 691 index is read from its JSON file into filename records.
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
    """Map GitHub Release asset filenames to SHA-256 digest values."""
    document = json.loads(path.read_text(encoding="utf-8"))
    return {
        asset["name"]: _digest_hex(asset.get("digest"))
        for asset in document.get("assets", [])
    }


def sha256_of(path: Path) -> str:
    """Return the lowercase SHA-256 digest of a file's bytes.

    Returns
    -------
    str
        The lowercase hexadecimal digest of the file contents.

    Examples
    --------
    Hashing a one-byte file returns its hexadecimal SHA-256 digest.
    """
    with path.open("rb") as handle:
        return hashlib.file_digest(handle, "sha256").hexdigest()


def local_names(directory: Path) -> list[str]:
    """List safe artefact filenames in a staging directory."""
    names = sorted(path.name for path in directory.iterdir() if path.is_file())
    unsafe = [name for name in names if _SAFE_NAME.fullmatch(name) is None]
    if unsafe:
        raise UnsafeArtifactNamesError(unsafe)
    return names


def _check_digest(path: Path, expected: str | None) -> None:
    """Raise unless a file has the expected SHA-256 digest."""
    actual = sha256_of(path)
    if expected is None or actual != expected:
        raise ArtifactDigestMismatchError(path.name, actual, expected)


def write_output(name: str, *, value: bool) -> None:
    """Append a lowercase Boolean workflow output to ``GITHUB_OUTPUT``."""
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        output.write(f"{name}={str(value).lower()}\n")


@dc.dataclass(frozen=True, slots=True)
class State:
    """The run's artefacts and the filenames held by each destination."""

    publish: Path
    pypi: dict[str, PypiFile]
    github: dict[str, str | None]

    @classmethod
    def load(cls, publish: Path, index: Path | None, assets: Path) -> State:
        """Read staged files, the optional PyPI index, and GitHub assets."""
        pypi = read_pypi_index(index) if index is not None else {}
        return cls(publish, pypi, read_github_assets(assets))

    def plan(self) -> Plan:
        """Plan this run's uploads against both destinations."""
        return plan(local_names(self.publish), self.pypi, self.github)


def carryover(state: State) -> list[str]:
    """Return names whose existing GitHub bytes can fill a PyPI gap."""
    return sorted(set(local_names(state.publish)) & set(state.github))


def stage_pypi(state: State, github_files: Path) -> bool:
    """Leave only missing canonical files in ``publish`` for PyPI upload.

    GitHub bytes take precedence over a rebuilt local file when PyPI lacks
    that filename. Existing PyPI names are removed from the upload directory.

    Returns
    -------
    bool
        Whether any file remains for the PyPI upload action.

    Examples
    --------
    A name already on PyPI is removed from the upload directory.
    """
    decided = state.plan()
    for name, source in sorted(decided.canonical.items()):
        artefact = state.publish / name
        if source is Source.PYPI:
            print(f"::notice::Skipping {name}: already on PyPI")
            artefact.unlink()
        elif source is Source.GITHUB:
            carried = github_files / name
            _check_digest(carried, state.github[name])
            shutil.copyfile(carried, artefact)
    if not decided.pypi_uploads:
        print("Every artefact is already on PyPI; nothing to publish.")
    return bool(decided.pypi_uploads)


def _pypi_fetch_list(
    lacking: cabc.Iterable[str], pypi: cabc.Mapping[str, PypiFile]
) -> list[tuple[str, str]]:
    """Return PyPI (filename, URL) pairs, refusing non-HTTPS URLs."""
    fetch = [(name, pypi[name].url) for name in lacking if name in pypi]
    if any(urllib.parse.urlparse(url).scheme != "https" for _, url in fetch):
        raise NonHttpsDownloadError
    return fetch


def _copy_pypi_gap(
    lacking: cabc.Iterable[str],
    pypi: cabc.Mapping[str, PypiFile],
    publish: Path,
    output: Path,
) -> None:
    """Copy locally built files PyPI does not yet list into ``output``."""
    for name in lacking:
        if name not in pypi:
            shutil.copyfile(publish / name, output / name)


def _copy_unattached_bundles(
    bundles: Path, github: cabc.Mapping[str, str | None], output: Path
) -> None:
    """Copy only provenance bundles the GitHub Release lacks."""
    for bundle in sorted(bundles.iterdir()):
        if bundle.is_file() and bundle.name not in github:
            shutil.copyfile(bundle, output / bundle.name)


def stage_github(
    state: State,
    bundles: Path,
    output: Path,
    github_files: Path | None = None,
) -> list[tuple[str, str]]:
    """Stage only missing GitHub assets and return canonical PyPI downloads.

    If ``github_files`` is provided, verify carried-over assets and refuse to
    proceed when their bytes disagree with PyPI's canonical bytes.

    Returns
    -------
    list[tuple[str, str]]
        The filenames and HTTPS URLs to fetch from PyPI before GitHub upload.

    Raises
    ------
    DestinationDigestMismatchError
        If an existing GitHub asset disagrees with PyPI's canonical bytes.

    Examples
    --------
    Missing PyPI names are copied locally; names already on PyPI are returned
    as HTTPS downloads for GitHub.
    """
    local = local_names(state.publish)
    if github_files is not None:
        for name in sorted(set(local) & set(state.github)):
            carried = github_files / name
            _check_digest(carried, state.github[name])
            pypi_file = state.pypi.get(name)
            if pypi_file is not None and sha256_of(carried) != pypi_file.sha256:
                raise DestinationDigestMismatchError(name)

    lacking = sorted(state.plan().github_uploads)
    fetch = _pypi_fetch_list(lacking, state.pypi)
    output.mkdir(parents=True, exist_ok=True)
    _copy_pypi_gap(lacking, state.pypi, state.publish, output)
    _copy_unattached_bundles(bundles, state.github, output)
    return fetch


def check_fetched(state: State, output: Path) -> None:
    """Raise unless every PyPI file staged for GitHub matches its digest."""
    for name in sorted(state.plan().github_uploads & set(state.pypi)):
        _check_digest(output / name, state.pypi[name].sha256)


def verify(state: State, bundles: Path) -> list[str]:
    """Describe artefacts or provenance bundles missing from either target."""
    pypi = {name: item.sha256 for name, item in state.pypi.items()}
    problems = mismatches(local_names(state.publish), pypi, state.github)
    missing = sorted(
        bundle.name
        for bundle in bundles.iterdir()
        if bundle.is_file() and bundle.name not in state.github
    )
    return [
        *problems,
        *(f"{name} is missing from the GitHub Release" for name in missing),
    ]
