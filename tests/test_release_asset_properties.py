"""Test release asset planning and mismatch invariants.

Run with ``pytest tests/test_release_asset_properties.py`` to check source
precedence, upload sets, and deterministic mismatch reports.
"""

from __future__ import annotations

from hypothesis import given
from hypothesis import strategies as st

from scripts.release_asset_plan import Source, mismatches, plan

_NAMES = st.sampled_from(["a.whl", "b.whl", "c.tar.gz", "d.whl"])
_DIGEST = st.sampled_from(["aa", "bb", "cc"])


@given(
    local=st.lists(_NAMES, max_size=12),
    pypi=st.sets(_NAMES),
    github=st.sets(_NAMES),
)
def test_plan_obeys_precedence_and_set_invariants(
    local: list[str], pypi: set[str], github: set[str]
) -> None:
    """Order and duplicates cannot change sources or destination uploads."""
    result = plan(local, pypi, github)
    names = set(local)
    expected_sources = {
        name: Source.PYPI
        if name in pypi
        else Source.GITHUB
        if name in github
        else Source.LOCAL
        for name in names
    }
    assert result.canonical == expected_sources, (
        "Canonical source must honour destination precedence."
    )
    assert result.pypi_uploads == names - pypi, (
        "PyPI receives exactly its missing names."
    )
    assert result.github_uploads == names - github, (
        "GitHub receives exactly its missing names."
    )
    assert plan(list(reversed(local)) + local, pypi, github) == result, (
        "Ordering and duplicates must not change the plan."
    )


@given(
    local=st.lists(_NAMES, max_size=12),
    pypi=st.dictionaries(_NAMES, _DIGEST),
    github=st.dictionaries(_NAMES, st.one_of(_DIGEST, st.none())),
)
def test_mismatches_are_sorted_and_invariant(
    local: list[str], pypi: dict[str, str], github: dict[str, str | None]
) -> None:
    """Each name yields the independently expected comparison in sorted order."""
    expected = []
    for name in sorted(set(local)):
        if name not in pypi:
            expected.append(f"{name} is missing from PyPI")
        elif github.get(name) is None:
            expected.append(
                f"{name} is missing from the GitHub Release or has no SHA-256 digest"
            )
        elif pypi[name] != github[name]:
            expected.append(
                f"{name} differs: PyPI sha256 {pypi[name]}, "
                f"GitHub sha256 {github[name]}"
            )
    assert mismatches(local, pypi, github) == expected, (
        "Report each mismatch in name order."
    )
    assert mismatches(list(reversed(local)) + local, pypi, github) == expected, (
        "Ordering and duplicates must not change mismatches."
    )
    equal = dict.fromkeys(set(local), "aa")
    assert mismatches(local, equal, equal) == [], (
        "Equal destination digests must match."
    )
