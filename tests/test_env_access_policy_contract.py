"""Contracts for the environment-access policy's build and CI wiring.

The Rust side of this policy is checked in `rust_extension/tests/`, where the
Clippy configuration and the compiled fixture live. Everything that is a fact
about the Makefile or the workflow is checked here instead, through the pinned
`makeutil` parser and PyYAML, so the repository keeps one Makefile parser
rather than two.

Two habits run through this module. Recipes are judged one whole command at a
time rather than by searching their text, because a substring search is
satisfied by `if false; then <command>; fi`. And a command's status is judged
as well as its text, because `-` prefixes and `|| true` leave a gate that runs,
can fail, and still reports success.
"""

from __future__ import annotations

import itertools
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the contract runs the real target
import tomllib
import typing as typ

import pytest
import yaml

from tests.make_contract_helpers import (
    mapping,
    objects,
    repo_root,
    sole_recipe_rule,
    sole_variable,
    text_sequence,
    variable_tokens,
    workflow_job,
)

if typ.TYPE_CHECKING:
    from pathlib import Path

type Tokens = tuple[str, ...]

#: The lint that carries the ban.
DISALLOWED_METHODS: typ.Final = "clippy::disallowed_methods"
#: Lanes required whatever else the list gains. `none` and `all` between them
#: compile both arms of every single-feature gate, which `--all-features`
#: alone cannot do.
REQUIRED_LANES: typ.Final[Tokens] = ("none", "all")
#: Manifest feature keys that name no lane of their own.
LANE_EXEMPT_FEATURES: typ.Final[Tokens] = ("default",)
#: The workflow that must run the gates, and the commands it must run.
CI_WORKFLOW: typ.Final = ".github/workflows/ci.yml"
RUST_TEST_COMMAND: typ.Final = (
    "cargo test --manifest-path rust_extension/Cargo.toml "
    "--no-default-features -- --test-threads=1"
)
REQUIRED_CI_COMMANDS: typ.Final[Tokens] = ("make lint", RUST_TEST_COMMAND)
#: Shell fragments that swallow the preceding command's exit status.
STATUS_SWALLOWING: typ.Final[Tokens] = (
    "|| true",
    "|| :",
    "|| /bin/true",
    "; true",
    "; :",
    "|| exit 0",
)


def sole_recipe(target: str) -> dict[str, object]:
    """Return the target's only recipe.

    Returns
    -------
    dict[str, object]
        Makeutil's fact for the recipe.
    """
    recipes = objects(sole_recipe_rule(target).get("recipes"), subject=f"{target}")
    assert len(recipes) == 1, f"{target} must be one command, found {len(recipes)}"
    return recipes[0]


def recipe_text(recipe: dict[str, object]) -> str:
    """Return a recipe's text with line continuations collapsed.

    Returns
    -------
    str
        The command on one line, with runs of whitespace collapsed.
    """
    text = recipe.get("text")
    assert isinstance(text, str), "makeutil must report recipe text as a string"
    return " ".join(text.replace("\\\n", " ").split())


def status_reaches_make(recipe: dict[str, object]) -> bool:
    """Report whether Make will see this command fail.

    A `-` prefix tells Make to ignore the exit status; the shell can discard it
    too. Either way the gate runs, can fail, and the target still succeeds.

    Returns
    -------
    bool
        True when a failure would fail the target.
    """
    if recipe.get("ignore_errors") is True:
        return False
    text = recipe_text(recipe)
    if any(text.endswith(suffix) for suffix in STATUS_SWALLOWING):
        return False
    return "|" not in text.replace("||", "")


def declared_features() -> Tokens:
    """Return the Cargo features that must each have a lane.

    Returns
    -------
    tuple[str, ...]
        Every declared feature except those exempt by name.
    """
    manifest = tomllib.loads(
        (repo_root() / "rust_extension" / "Cargo.toml").read_text(encoding="utf-8")
    )
    features = manifest.get("features", {})
    return tuple(name for name in features if name not in LANE_EXEMPT_FEATURES)


def test_policy_lanes_cover_both_arms_of_every_feature_gate() -> None:
    """Scenario: a contributor adds a Cargo feature or edits the lane list.

    Invariant: `none` and `all` are present, and every declared feature has a
    lane of its own. A feature added without a lane fails this test rather than
    going unlinted.
    """
    lanes = variable_tokens("ENV_POLICY_FEATURE_LANES")
    for required in REQUIRED_LANES:
        assert required in lanes, (
            f"ENV_POLICY_FEATURE_LANES must include the {required!r} lane, "
            f"found {lanes}"
        )
    for feature in declared_features():
        assert feature in lanes, (
            f"ENV_POLICY_FEATURE_LANES must lint the {feature!r} feature, found {lanes}"
        )


def test_policy_lint_reaches_every_target_kind_and_denies_the_lint() -> None:
    """Scenario: a contributor edits the policy lane's arguments.

    Invariant: the lint still covers every target kind, so tests and benches
    are governed, and still denies the lint outright rather than warning.
    """
    cargo_args = variable_tokens("ENV_POLICY_CARGO_ARGS")
    assert "--all-targets" in cargo_args, (
        f"ENV_POLICY_CARGO_ARGS must lint every target kind, found {cargo_args}"
    )
    lint_args = variable_tokens("ENV_POLICY_LINT_ARGS")
    denials = list(itertools.pairwise(lint_args))
    assert ("-D", DISALLOWED_METHODS) in denials, (
        f"ENV_POLICY_LINT_ARGS must deny {DISALLOWED_METHODS}, found {lint_args}"
    )


def test_policy_recipe_drives_the_lane_script_with_every_input() -> None:
    """Scenario: a contributor edits the policy recipe.

    Invariant: it is one command that hands the lane driver its lane list and
    both argument sets, ends in the driver call, and lets Make see it fail.
    Requiring the command to *end* in the driver call is what rejects a
    wrapper, whose command ends with `fi`.
    """
    recipe = sole_recipe("lint-env-policy")
    text = recipe_text(recipe)
    for exported in (
        'INPUT_LANES="$(ENV_POLICY_FEATURE_LANES)"',
        'INPUT_CARGO_ARGS="$(ENV_POLICY_CARGO_ARGS)"',
        'INPUT_LINT_ARGS="$(ENV_POLICY_LINT_ARGS)"',
    ):
        assert exported in text, f"lint-env-policy must export {exported}"
    assert text.endswith("uv run --script $(LINT_LANES_SCRIPT)"), (
        f"lint-env-policy must end in the lane driver call, found {text!r}"
    )
    assert status_reaches_make(recipe), (
        f"lint-env-policy must let Make see the driver fail, found {text!r}"
    )
    script = sole_variable("LINT_LANES_SCRIPT").get("raw_value")
    assert script == "scripts/lint_rust_lanes.py", (
        f"LINT_LANES_SCRIPT must name the lane driver, found {script!r}"
    )


def runs_target(parent: str, child: str) -> bool:
    """Report whether `parent` runs `child` by an unwrappable route.

    A prerequisite cannot be wrapped or made conditional at all. A recipe
    command can, so it is matched whole and its status checked.

    Returns
    -------
    bool
        True when the child is reached unconditionally.
    """
    rule = sole_recipe_rule(parent, require_recipes=False)
    prerequisites = text_sequence(rule.get("prerequisites"), subject=f"{parent}")
    if child in prerequisites:
        return True
    recipes = objects(rule.get("recipes"), subject=f"{parent}")
    delegation = f"$(MAKE) {child}"
    return any(
        recipe_text(recipe) == delegation and status_reaches_make(recipe)
        for recipe in recipes
    )


def test_make_lint_reaches_the_policy_lane() -> None:
    """Scenario: a contributor restructures the lint targets.

    Invariant: `make lint` still reaches the policy lane, and `lint-rust` still
    proves the lane driver before trusting it.
    """
    prerequisites = text_sequence(
        sole_recipe_rule("lint-rust", require_recipes=False).get("prerequisites"),
        subject="lint-rust",
    )
    for required in ("lint-env-policy", "lint-lanes-test"):
        assert required in prerequisites, (
            f"lint-rust must run {required}, found {prerequisites}"
        )
    assert runs_target("lint", "lint-rust"), (
        "lint must run lint-rust as a prerequisite, or as a command of its own "
        "whose failure reaches Make"
    )


def workflow_steps(job_name: str) -> list[dict[str, object]]:
    """Return a workflow job's steps.

    Returns
    -------
    list[dict[str, object]]
        Every step in the job, in order.
    """
    job = workflow_job(CI_WORKFLOW, job_name)
    return objects(job.get("steps"), subject=f"{job_name} steps")


def job_tolerance_is_acceptable(job: dict[str, object]) -> bool:
    """Report whether a job's failure tolerance is acceptable for a gate.

    An allow-list of shapes, not a deny-list of spellings: a deny-list would
    have to enumerate every constant-true expression. A job may omit the key,
    set an explicit `false`, or consult the matrix, which is how an
    experimental leg is singled out.

    Returns
    -------
    bool
        True when the job's failure would still fail CI.
    """
    if "continue-on-error" not in job:
        return True
    tolerance = job["continue-on-error"]
    if tolerance is False:
        return True
    return isinstance(tolerance, str) and "matrix." in tolerance


def step_tolerance_is_acceptable(step: dict[str, object]) -> bool:
    """Report whether a step's failure tolerance is acceptable for a gate.

    Stricter than the job rule. A matrix expression on a job says which leg is
    experimental; on a required step it would say this gate may fail.

    Returns
    -------
    bool
        True when the step's failure would still fail its job.
    """
    return "continue-on-error" not in step or step["continue-on-error"] is False


def test_ci_triggers_on_pull_requests() -> None:
    """Scenario: a contributor edits the workflow triggers.

    Invariant: the workflow still fires on pull requests. Without it every step
    below remains correct and none of them runs.
    """
    workflow = mapping(
        yaml.safe_load((repo_root() / CI_WORKFLOW).read_text(encoding="utf-8")),
        subject=CI_WORKFLOW,
    )
    triggers = mapping(workflow.get(True, workflow.get("on")), subject="triggers")
    assert "pull_request" in triggers, f"{CI_WORKFLOW} must trigger on pull_request"


@pytest.mark.parametrize("command", REQUIRED_CI_COMMANDS)
def test_ci_runs_each_policy_gate_unconditionally(command: str) -> None:
    """Scenario: a contributor edits the workflow's gate steps.

    Invariant: each gate is some step's whole `run` value, and neither that
    step nor its job carries a condition or tolerates its own failure. The
    value of a condition is never inspected: `if: false` and a push-only
    condition skip the gate equally, and YAML resolves `false` to a boolean
    whose string form is `False`, so no list of spellings is reliable.
    """
    job_name = "build-test"
    job = workflow_job(CI_WORKFLOW, job_name)
    matches = [step for step in workflow_steps(job_name) if step.get("run") == command]
    assert matches, f"{CI_WORKFLOW} must run {command!r} as a step's whole command"
    step = matches[0]
    assert "if" not in step, (
        f"the {command!r} step in job {job_name} must carry no condition"
    )
    assert "if" not in job, f"job {job_name} runs {command!r} but carries a condition"
    assert step_tolerance_is_acceptable(step), (
        f"the {command!r} step in job {job_name} must not tolerate its own failure"
    )
    assert job_tolerance_is_acceptable(job), (
        f"job {job_name} runs {command!r} but tolerates failure outside a matrix leg"
    )


def run_policy_lane(lanes: str) -> int:
    """Run `make lint-env-policy` over the given lanes.

    The lint flags are narrowed so the run exercises the recipe's exit-status
    handling rather than the policy itself. A lane naming a feature the crate
    does not declare fails immediately, before any build.

    Returns
    -------
    int
        The target's exit code.
    """
    make = shutil.which("make")
    assert make is not None, "make must be on PATH to run the policy target"
    # ruff: ignore[subprocess-without-shell-equals-true] the argument vector is
    # fixed here and the executable is resolved above
    return subprocess.run(
        [
            make,
            "lint-env-policy",
            f"ENV_POLICY_FEATURE_LANES={lanes}",
            "ENV_POLICY_LINT_ARGS=-A clippy::all",
        ],
        cwd=repo_root(),
        capture_output=True,
        check=False,
    ).returncode


@pytest.mark.timeout(120)
def test_a_failing_lane_fails_the_policy_target() -> None:
    """Scenario: one lane rejects the code and a later lane accepts it.

    Invariant: `make lint-env-policy` fails. This drives the real recipe rather
    than describing it, because the shell loop it replaced reported only its
    last command's status, and a contract that reads the recipe cannot tell the
    difference. `badfeature` is not a declared feature, so Cargo rejects that
    lane before building anything.
    """
    assert run_policy_lane("badfeature none") != 0, (
        "a failing lane must fail lint-env-policy, even when a later lane passes"
    )
    assert run_policy_lane("none") == 0, (
        "lint-env-policy must succeed when every lane passes"
    )


#: Attributes that would switch the policy off wholesale. `clippy::all` and
#: `warnings` are included because either silences the policy lint along with
#: everything else.
POLICY_DISABLING_LINTS: typ.Final[Tokens] = (
    DISALLOWED_METHODS,
    "clippy::all",
    "warnings",
)


def rust_sources() -> list[Path]:
    """Return every Rust source file in the extension.

    Returns
    -------
    list[Path]
        Sources under `src`, `tests` and `benches`.
    """
    crate = repo_root() / "rust_extension"
    return sorted(
        path
        for directory in ("src", "tests", "benches")
        for path in (crate / directory).rglob("*.rs")
    )


def allow_attributes(source: str) -> list[str]:
    """Return every `allow` attribute written at the start of a line.

    Only a line that *begins* with the attribute counts. An attribute quoted
    inside a doc comment or a string is discussion, not policy, and this file's
    own mutation records quote attributes. Continuation lines are joined, since
    rustfmt wraps a long attribute across several.

    Returns
    -------
    list[str]
        Each attribute on one line.
    """
    found: list[str] = []
    pending = ""
    for line in source.splitlines():
        stripped = line.strip()
        if not pending and not stripped.startswith(("#[allow(", "#![allow(")):
            continue
        pending = f"{pending} {stripped}".strip()
        if pending.count("(") <= pending.count(")"):
            found.append(" ".join(pending.split()))
            pending = ""
    return found


def test_no_source_switches_the_policy_off_with_an_allow() -> None:
    """Scenario: a contributor silences the policy with an `allow` attribute.

    Invariant: no Rust source allows the policy lint, `clippy::all`, or
    `warnings`, inner or outer.

    A crate-level inner attribute is the dangerous one and no other contract
    sees it. `#![allow(clippy::disallowed_methods, reason = "...")]` at the top
    of a file disables the policy for that whole crate while the Clippy
    configuration, the manifest severity, the lane list and the compiled
    fixture all stay exactly as they are, and every one of their contracts
    stays green. `clippy::allow_attributes`, which would otherwise object to an
    `allow` where an `expect` belongs, does not fire on inner attributes.

    The sanctioned exception is `expect`, not `allow`, and is item-scoped; that
    shape is checked separately.

    Mutation proof (2026-09-08): adding
    `#![allow(clippy::disallowed_methods, reason = "...")]` to
    `rust_extension/src/lib.rs` with a real `std::env::var` call beneath it
    failed only this test. The four Rust contracts passed and `make
    lint-env-policy` exited 0, which is exactly the hole. An item-level
    `#[allow(warnings, ...)]`, and the inner attribute wrapped across lines as
    rustfmt writes it, each failed here too.
    """
    offences = [
        f"{path.relative_to(repo_root())}: {attribute}"
        for path in rust_sources()
        for attribute in allow_attributes(path.read_text(encoding="utf-8"))
        if any(lint in attribute for lint in POLICY_DISABLING_LINTS)
    ]
    assert not offences, (
        "no Rust source may switch the environment-access policy off with an "
        f"allow; use an item-scoped expect with a reason instead: {offences}"
    )
