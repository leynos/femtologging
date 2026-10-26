"""Choose canonical release bytes and calculate missing asset names.

The workflow uses these network-free decisions before staging files for PyPI
and the tag's GitHub Release.
"""

from __future__ import annotations

import dataclasses as dc
import enum
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc


class Source(enum.StrEnum):
    """Destination supplying the canonical bytes for one artefact name."""

    PYPI = "pypi"
    GITHUB = "github"
    LOCAL = "local"


@dc.dataclass(frozen=True, slots=True)
class Plan:
    """Canonical sources and missing names for both destinations.

    Attributes
    ----------
    canonical
        Source of canonical bytes for each filename built in this run.
    pypi_uploads
        Local filenames that PyPI does not already hold.
    github_uploads
        Local filenames that the GitHub Release does not already hold.
    """

    canonical: cabc.Mapping[str, Source]
    pypi_uploads: frozenset[str]
    github_uploads: frozenset[str]


def _canonical_source(
    name: str,
    on_pypi: cabc.Collection[str],
    on_github: cabc.Collection[str],
) -> Source:
    """Return the highest-priority destination that already holds ``name``."""
    if name in on_pypi:
        return Source.PYPI
    if name in on_github:
        return Source.GITHUB
    return Source.LOCAL


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
    canonical = {name: _canonical_source(name, on_pypi, on_github) for name in names}
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
