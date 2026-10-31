"""Contracts for the Dependabot configuration of the extension build.

maturin (the ``uv`` ecosystem) and PyO3 (the ``cargo`` ecosystem) are versioned
independently yet build one extension, so an update to either alone cannot be
built against the other. A multi-ecosystem group puts the available updates for
the two into one pull request, so when both have one a single CI run builds
both. It does not hold back a bump when only one of the two has an update, and a
group coordinates the update without proving compatibility.

Dependabot builds one job per entry, and an entry that joins a group runs only
that group, over the dependencies its ``patterns`` take (cuprum's first run of
the arrangement showed it: the entry's own groups, ignore rules and every other
dependency were not processed). So each of the two ecosystems has two entries,
the group entry and a plain entry that ignores the pair so that it is handled
once.

The project is uv-managed (``uv.lock``, no requirements files), so the Python
entries must be ``uv``: a ``pip`` entry leaves the lock outside Dependabot's
view. A mistake in this file is silent, because Dependabot reports nothing for
an entry that names a group the file never defines, so the checked-in
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


def _entries() -> list[dict[str, typ.Any]]:
    """Return every ``updates`` entry, in order."""
    updates = _document().get("updates")
    assert isinstance(updates, list), "the configuration must declare `updates`"
    return typ.cast("list[dict[str, typ.Any]]", updates)


def _group_entries() -> list[dict[str, typ.Any]]:
    """Return the entries that join a multi-ecosystem group."""
    return [entry for entry in _entries() if "multi-ecosystem-group" in entry]


def _plain_entry(ecosystem: str) -> dict[str, typ.Any]:
    """Return the one entry for *ecosystem* that does not join a group."""
    plain = [
        entry
        for entry in _entries()
        if entry["package-ecosystem"] == ecosystem
        and "multi-ecosystem-group" not in entry
    ]
    assert len(plain) == 1, f"{ecosystem} needs exactly one plain entry"
    return plain[0]


def test_the_python_entries_are_uv_not_pip() -> None:
    """Watch the uv-managed project through the ecosystem that reads ``uv.lock``."""
    ecosystems = {entry["package-ecosystem"] for entry in _entries()}
    assert "uv" in ecosystems, (
        f"the Python entries must be `uv`; got {sorted(ecosystems)}"
    )
    assert "pip" not in ecosystems, "a `pip` entry leaves `uv.lock` unwatched"


def test_pytest_bdd_is_updated_by_hand() -> None:
    """Hold pytest-bdd back: it is pinned beside pytest and checked against it."""
    ignored = [rule["dependency-name"] for rule in _plain_entry("uv")["ignore"]]
    assert "pytest-bdd" in ignored, (
        f"the plain uv entry must ignore pytest-bdd; got {ignored}"
    )


def test_the_group_is_defined_with_a_daily_schedule() -> None:
    """Define the group the two entries join, on the estate's daily cadence."""
    groups = _document().get("multi-ecosystem-groups")
    assert isinstance(groups, dict), "`multi-ecosystem-groups` must be defined"
    assert GROUP in groups, f"{GROUP!r} must be defined; got {sorted(groups)}"
    interval = groups[GROUP].get("schedule", {}).get("interval")
    assert interval == "daily", f"the {GROUP} group must run daily; got {interval!r}"


def test_the_group_carries_the_pull_request_bound_not_its_entries() -> None:
    """Bound the group; Dependabot rejects the bound on an entry that joins it."""
    limit = _document()["multi-ecosystem-groups"][GROUP].get("open-pull-requests-limit")
    assert isinstance(limit, int), (
        f"the {GROUP} group needs an integer bound; got {limit!r}"
    )
    assert limit > 0, f"the {GROUP} group needs a positive bound; got {limit!r}"
    carrying = [
        entry["package-ecosystem"]
        for entry in _group_entries()
        if "open-pull-requests-limit" in entry
    ]
    assert not carrying, (
        f"Dependabot rejects the bound on {carrying}; set it on the group"
    )


@pytest.mark.parametrize(("ecosystem", "members"), sorted(MEMBERS.items()))
def test_each_half_of_the_extension_build_has_one_group_entry(
    ecosystem: str, members: list[str]
) -> None:
    """Name the group and the member in one entry per ecosystem, with no groups."""
    entries = [e for e in _group_entries() if e["package-ecosystem"] == ecosystem]
    assert len(entries) == 1, (
        f"{ecosystem} needs exactly one entry joining {GROUP!r}; got {len(entries)}"
    )
    (entry,) = entries
    assert entry["multi-ecosystem-group"] == GROUP, (
        f"the {ecosystem} group entry must join {GROUP!r}"
    )
    assert entry.get("patterns") == members, (
        f"the {ecosystem} group entry must offer exactly {members}; "
        f"got {entry.get('patterns')!r}"
    )
    assert "groups" not in entry, (
        f"the {ecosystem} group entry runs only the group, so it carries no `groups`"
    )


@pytest.mark.parametrize(("ecosystem", "members"), sorted(MEMBERS.items()))
def test_the_plain_entry_ignores_what_the_group_entry_handles(
    ecosystem: str, members: list[str]
) -> None:
    """Keep the pair out of the plain entry, which handles everything else."""
    ignored = [rule["dependency-name"] for rule in _plain_entry(ecosystem)["ignore"]]
    missing = [name for name in members if name not in ignored]
    assert not missing, f"the plain {ecosystem} entry must ignore {missing}"


def test_only_the_extension_build_ecosystems_join_a_group() -> None:
    """Keep the group to the two halves of the extension build."""
    joined = {entry["package-ecosystem"] for entry in _group_entries()}
    assert joined == set(MEMBERS), (
        f"only {sorted(MEMBERS)} may join {GROUP!r}; got {sorted(joined)}"
    )
