"""Contracts for the Dependabot configuration of the extension build.

maturin (the ``uv`` stanza) and PyO3 (the ``cargo`` stanza) are versioned
independently yet build one extension, so an update to either alone cannot be
built against the other. A multi-ecosystem group puts the available updates for
the two into one pull request, so when both have one a single CI run builds
both. It does not hold back a bump when only one of the two has an update, and
a group coordinates the update without proving compatibility.

The project is uv-managed (``uv.lock``, no requirements files), so the Python
stanza must be ``uv``: a ``pip`` stanza leaves the lock outside Dependabot's
view. A mistake in this file is silent, because Dependabot reports nothing for
a stanza that names a group the file never defines, so the checked-in
configuration is read back and held to the arrangement.
"""

from __future__ import annotations

import typing as typ

import pytest
import yaml

from tests.make_contract_helpers import repo_root

CONFIG: typ.Final = repo_root() / ".github" / "dependabot.yml"

#: The group the two stanzas join.
GROUP: typ.Final = "maturin-pyo3"

#: What each ecosystem offers to that group.
MEMBERS: typ.Final = {"uv": ["maturin"], "cargo": ["pyo3"]}


def _document() -> dict[str, typ.Any]:
    """Return the parsed Dependabot configuration."""
    document = yaml.safe_load(CONFIG.read_text(encoding="utf-8"))
    assert isinstance(document, dict), f"{CONFIG} must parse to a mapping"
    return typ.cast("dict[str, typ.Any]", document)


def _stanzas() -> dict[str, dict[str, typ.Any]]:
    """Return each update stanza keyed by its package ecosystem."""
    updates = _document().get("updates")
    assert isinstance(updates, list), "the configuration must declare `updates`"
    return {str(stanza["package-ecosystem"]): stanza for stanza in updates}


def test_the_python_stanza_is_uv_not_pip() -> None:
    """Watch the uv-managed project through the ecosystem that reads ``uv.lock``."""
    ecosystems = set(_stanzas())
    assert "uv" in ecosystems, (
        f"the Python stanza must be `uv`; got {sorted(ecosystems)}"
    )
    assert "pip" not in ecosystems, "a `pip` stanza leaves `uv.lock` unwatched"


def test_pytest_bdd_is_updated_by_hand() -> None:
    """Hold pytest-bdd back: it is pinned beside pytest and checked against it."""
    ignored = [
        rule.get("dependency-name") for rule in _stanzas()["uv"].get("ignore", [])
    ]
    assert ignored == ["pytest-bdd"], (
        f"the uv stanza must ignore exactly pytest-bdd; got {ignored}"
    )


def test_the_group_carries_the_pull_request_bound_not_its_entries() -> None:
    """Bound the group; Dependabot rejects the bound on an entry that joins it."""
    limit = _document()["multi-ecosystem-groups"][GROUP].get("open-pull-requests-limit")
    assert isinstance(limit, int), (
        f"the {GROUP} group needs an integer bound; got {limit!r}"
    )
    assert limit > 0, f"the {GROUP} group needs a positive bound; got {limit!r}"
    carrying = [
        ecosystem
        for ecosystem, stanza in _stanzas().items()
        if "multi-ecosystem-group" in stanza and "open-pull-requests-limit" in stanza
    ]
    assert not carrying, (
        f"Dependabot rejects the bound on {carrying}; set it on the group"
    )


def test_the_group_is_defined_with_a_daily_schedule() -> None:
    """Define the group the two stanzas join, on the estate's daily cadence."""
    groups = _document().get("multi-ecosystem-groups")
    assert isinstance(groups, dict), "`multi-ecosystem-groups` must be defined"
    assert GROUP in groups, f"{GROUP!r} must be defined; got {sorted(groups)}"
    interval = groups[GROUP].get("schedule", {}).get("interval")
    assert interval == "daily", f"the {GROUP} group must run daily; got {interval!r}"


@pytest.mark.parametrize(("ecosystem", "members"), sorted(MEMBERS.items()))
def test_each_half_of_the_extension_build_joins_the_group(
    ecosystem: str, members: list[str]
) -> None:
    """Name the group and the members in the stanza, or the pair arrives split."""
    stanza = _stanzas()[ecosystem]
    assert stanza.get("multi-ecosystem-group") == GROUP, (
        f"the {ecosystem} stanza must join {GROUP!r}; "
        f"got {stanza.get('multi-ecosystem-group')!r}"
    )
    assert stanza.get("patterns") == members, (
        f"the {ecosystem} stanza must offer exactly {members} to the group; "
        f"got {stanza.get('patterns')!r}"
    )


def test_no_other_stanza_joins_a_multi_ecosystem_group() -> None:
    """Keep the group to the two halves of the extension build."""
    joined = {
        ecosystem
        for ecosystem, stanza in _stanzas().items()
        if "multi-ecosystem-group" in stanza
    }
    assert joined == set(MEMBERS), (
        f"only {sorted(MEMBERS)} may join {GROUP!r}; got {sorted(joined)}"
    )
