"""Reconcile release artefacts between PyPI and the GitHub Release.

For each filename, PyPI's bytes are canonical first, followed by an existing
GitHub Release asset, and then this run's build. Each destination receives
only missing names. This module has no network access; the workflow performs
all requests through ``curl`` and ``gh``.
"""

from __future__ import annotations

import dataclasses as dc
import shutil
import typing as typ
import urllib.parse

from scripts.release_asset_errors import (
    ArtifactDigestMismatchError,
    DestinationDigestMismatchError,
    NonHttpsDownloadError,
    ReleaseAssetError,
)
from scripts.release_asset_io import (
    INDEX_URL,
    PypiFile,
    local_names,
    read_github_assets,
    read_pypi_index,
    sha256_of,
    write_output,
)
from scripts.release_asset_plan import Plan, Source, mismatches, plan

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

__all__ = [
    "INDEX_URL",
    "Plan",
    "PypiFile",
    "ReleaseAssetError",
    "Source",
    "State",
    "carryover",
    "check_fetched",
    "local_names",
    "mismatches",
    "plan",
    "read_github_assets",
    "read_pypi_index",
    "sha256_of",
    "stage_github",
    "stage_pypi",
    "verify",
    "write_output",
]


def _check_digest(path: Path, expected: str | None) -> None:
    """Raise unless a file has the expected SHA-256 digest."""
    actual = sha256_of(path)
    if expected is None or actual != expected:
        raise ArtifactDigestMismatchError(path.name, actual, expected)


@dc.dataclass(frozen=True, slots=True)
class State:
    """Local artefacts and the filenames held by each destination.

    Attributes
    ----------
    publish
        Directory holding this run's current wheel and sdist files.
    pypi
        PyPI filenames mapped to their canonical digest and download URL.
    github
        GitHub Release filenames mapped to their reported SHA-256 digest.
    """

    publish: Path
    pypi: dict[str, PypiFile]
    github: dict[str, str | None]

    @classmethod
    def load(cls, publish: Path, index: Path | None, assets: Path) -> State:
        """Load the local artefacts and destination snapshots.

        Parameters
        ----------
        publish
            Directory containing this run's wheel and sdist files.
        index
            Optional PyPI Simple index JSON path. Pass ``None`` when staging
            must consider only GitHub's existing assets.
        assets
            GitHub Release asset snapshot in ``gh release view`` JSON format.

        Returns
        -------
        State
            A snapshot used by the reconciliation operations.

        Examples
        --------
        ``State.load(Path("dist/publish"), Path("index.json"), Path("assets.json"))``
        reads the three snapshots needed for release reconciliation.
        """
        pypi = read_pypi_index(index) if index is not None else {}
        return cls(publish, pypi, read_github_assets(assets))

    def plan(self) -> Plan:
        """Plan uploads using the current PyPI and GitHub snapshots.

        Returns
        -------
        Plan
            Canonical bytes and the names missing from each destination.

        Examples
        --------
        A state with no existing assets plans every local artefact for both.
        """
        return plan(local_names(self.publish), self.pypi, self.github)


def carryover(state: State) -> list[str]:
    """List built filenames already present on the GitHub Release.

    Parameters
    ----------
    state
        Current local file list and GitHub asset snapshot.

    Returns
    -------
    list[str]
        Sorted names whose GitHub bytes may be used when PyPI lacks them.

    Examples
    --------
    An existing GitHub wheel name is returned when the same wheel is local.
    """
    return sorted(set(local_names(state.publish)) & set(state.github))


def stage_pypi(state: State, github_files: Path) -> bool:
    """Leave only missing canonical files in ``publish`` for PyPI upload.

    GitHub bytes take precedence over a rebuilt local file when PyPI lacks
    that filename. Existing PyPI names are removed from the upload directory.

    Parameters
    ----------
    state
        Current local file list and destination snapshots.
    github_files
        Directory containing downloaded GitHub assets for carry-over.

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

    Parameters
    ----------
    state
        Current local file list and destination snapshots.
    bundles
        Directory containing this run's provenance bundle.
    output
        Destination directory for files that GitHub still needs.
    github_files
        Optional directory containing previously downloaded GitHub assets.

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
    """Check each PyPI download staged for the GitHub Release.

    Parameters
    ----------
    state
        Current local file list and PyPI digest snapshot.
    output
        Directory containing the files downloaded from PyPI.

    Examples
    --------
    The check succeeds when every staged file matches the PyPI index hash.
    """
    for name in sorted(state.plan().github_uploads & set(state.pypi)):
        _check_digest(output / name, state.pypi[name].sha256)


def verify(state: State, bundles: Path) -> list[str]:
    """List artefacts or bundles missing from the reconciled destinations.

    Parameters
    ----------
    state
        Current local files and the latest PyPI and GitHub snapshots.
    bundles
        Directory containing this run's provenance bundles.

    Returns
    -------
    list[str]
        Human-readable missing-name and digest-mismatch descriptions. An empty
        list means every release artefact is present with matching digests.

    Examples
    --------
    A fully reconciled release returns an empty list of problems.
    """
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
