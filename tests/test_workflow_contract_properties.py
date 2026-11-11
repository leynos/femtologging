"""Property tests for the Make and workflow contract readers.

Each property compares a reader with an independent model of the same rule, so
a change to a reader cannot silently agree with its own helpers. The models are
deliberately small: breadth-first reachability, ordered accumulation, and a
plain reading of the provisioning rules.
"""

from __future__ import annotations

import copy
import typing as typ

from hypothesis import given, settings
from hypothesis import strategies as st

from tests.make_contract_helpers import prerequisite_map
from tests.workflow_contract_helpers import (
    PREFLIGHT,
    depends_on_preflight,
    provisioning_problems,
)

if typ.TYPE_CHECKING:
    from tests.workflow_document_helpers import Job

SETUP_RUST: typ.Final[str] = (
    "leynos/shared-actions/.github/actions/setup-rust@"
    "ff1dd759dfffc0db3459e30e833f52437ee62b57"
)
MAKE_SUITE: typ.Final[str] = "make test"
CARGO_SUITE: typ.Final[str] = "cargo test --manifest-path rust_extension/Cargo.toml"
SUITE_RUNS: typ.Final[frozenset[str]] = frozenset({MAKE_SUITE, CARGO_SUITE})
TARGETS: typ.Final[tuple[str, ...]] = (PREFLIGHT, "a", "b", "c", "d")
STEP_KINDS: typ.Final[tuple[str, ...]] = (
    "install",
    "conditional-install",
    "soft-failing-install",
    "setup-rust-without-mold",
    "make-suite",
    "cargo-suite",
    "unrelated",
)


def _reachable(graph: dict[str, list[str]], target: str) -> bool:
    """Report whether a path of length at least one reaches the preflight."""
    frontier = list(graph.get(target, ()))
    visited = {target}
    while frontier:
        current = frontier.pop()
        if current == PREFLIGHT:
            return True
        if current in visited:
            continue
        visited.add(current)
        frontier.extend(graph.get(current, ()))
    return False


def _model_prerequisite_map(
    rules: list[dict[str, typ.Any]],
) -> dict[str, tuple[str, ...]]:
    """Return a target's prerequisites, unioned across rules in order."""
    merged: dict[str, list[str]] = {}
    for rule in rules:
        for target in rule["targets"]:
            merged.setdefault(target, []).extend(rule["prerequisites"])
    return {target: tuple(names) for target, names in merged.items()}


_STEP_TEMPLATES: typ.Final[dict[str, Job]] = {
    "install": {
        "name": "Install rust",
        "uses": SETUP_RUST,
        "with": {"install-mold": "true"},
    },
    "conditional-install": {
        "name": "Install rust",
        "uses": SETUP_RUST,
        "with": {"install-mold": "true"},
        "if": "always()",
    },
    "soft-failing-install": {
        "name": "Install rust",
        "uses": SETUP_RUST,
        "with": {"install-mold": "true"},
        "continue-on-error": True,
    },
    "setup-rust-without-mold": {"name": "Install rust", "uses": SETUP_RUST},
    "make-suite": {"name": "Test", "run": MAKE_SUITE},
    "cargo-suite": {"name": "Test", "run": CARGO_SUITE},
    "unrelated": {"name": "Checkout", "uses": "actions/checkout@v7"},
}


def _step(kind: str) -> Job:
    """Return a fresh workflow step of `kind` from the provisioning alphabet."""
    return copy.deepcopy(_STEP_TEMPLATES[kind])


def _is_install_step(step: Job) -> bool:
    """Report whether the model counts `step` as installing mold."""
    settings = step.get("with") or {}
    installs_mold = str(settings.get("install-mold")).lower() == "true"
    return "setup-rust" in str(step.get("uses", "")) and installs_mold


def _is_suite_step(step: Job) -> bool:
    """Report whether the model counts `step` as running the suite."""
    run = str(step.get("run", ""))
    return "generate-coverage" in str(step.get("uses", "")) or run in SUITE_RUNS


def _model_is_provisioned(steps: list[Job], *, is_linux: bool) -> bool:
    """Report whether the stated provisioning rules are satisfied."""
    suite_at = [at for at, step in enumerate(steps) if _is_suite_step(step)]
    if not is_linux or not suite_at:
        return True
    installs = [at for at, step in enumerate(steps) if _is_install_step(step)]
    has_early_install = any(at < suite_at[0] for at in installs)
    all_sound = all(
        "if" not in steps[at] and not steps[at].get("continue-on-error")
        for at in installs
    )
    return has_early_install and all_sound


GRAPHS: typ.Final = st.dictionaries(
    keys=st.sampled_from(TARGETS),
    values=st.lists(st.sampled_from(TARGETS), max_size=4),
    max_size=5,
)
RULE_LISTS: typ.Final = st.lists(
    st.fixed_dictionaries({
        "targets": st.lists(st.sampled_from(TARGETS), min_size=1, max_size=2),
        "prerequisites": st.lists(st.sampled_from(TARGETS), max_size=3),
    }),
    max_size=4,
)
STEP_LISTS: typ.Final = st.lists(st.sampled_from(STEP_KINDS).map(_step), max_size=6)


@settings(max_examples=50, deadline=None)
@given(graph=GRAPHS, rules=RULE_LISTS)
def test_prerequisite_readers_match_independent_models(
    graph: dict[str, list[str]], rules: list[dict[str, typ.Any]]
) -> None:
    """Reachability and accumulation agree with their own simple models."""
    depends = depends_on_preflight(graph)
    for target in TARGETS:
        assert depends(target) is _reachable(graph, target), target
    assert prerequisite_map(rules) == _model_prerequisite_map(rules), (
        f"accumulation must match the model for {rules}"
    )


@settings(max_examples=50, deadline=None)
@given(
    steps=STEP_LISTS,
    runner=st.sampled_from(("ubuntu-latest", "windows-2022")),
)
def test_provisioning_matches_the_stated_rules(steps: list[Job], runner: str) -> None:
    """The provisioner reports a problem exactly when the model says it must."""
    job: Job = {"runs-on": runner, "steps": steps}
    depends = depends_on_preflight({"test": (PREFLIGHT,)})
    problems = provisioning_problems(job, depends)
    modelled = _model_is_provisioned(steps, is_linux=runner == "ubuntu-latest")
    assert bool(problems) is not modelled, (steps, runner, problems)
