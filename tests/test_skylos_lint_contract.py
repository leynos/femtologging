"""Contract tests for Skylos dead-code detection in Make and CI.

Skylos is invoked through variables and recipes whose order is significant:
the scanner accepts ``--config-file`` before a scan path, while the standalone
``whitelist`` subcommand must appear immediately after ``skylos``. Skylos also
parses source with its own runtime AST, so it must run under Python 3.14 to
stay ahead of the project syntax. Makeutil parses the Makefile into structured
rules and variables, so these tests assert that interface without depending on
whitespace or nearby source text.
"""

from __future__ import annotations

import shlex
import tomllib
import typing as typ

from tests.make_contract_helpers import (
    mapping,
    objects,
    recipe_tokens,
    repo_root,
    sole_recipe_rule,
    sole_workflow_step,
    text_sequence,
    variable_tokens,
    workflow_job,
)

_MAKEUTIL_REVISION: typ.Final = "29fc5a1634ffbaa18a773eed9dff1b2838a45d9c"
_MAKEUTIL_TOOLCHAIN: typ.Final = "nightly-2026-05-28"
_MAKEUTIL_INSTALL_TOKENS: typ.Final = (
    "rustup",
    "toolchain",
    "install",
    "${MAKEUTIL_TOOLCHAIN}",
    "--profile",
    "minimal",
    "RUSTFLAGS=-Zpolonius=next",
    "cargo",
    "+${MAKEUTIL_TOOLCHAIN}",
    "install",
    "--git",
    "https://github.com/leynos/makeutil",
    "--rev",
    "${MAKEUTIL_REVISION}",
    "--locked",
    "--force",
    "makeutil",
)
_SKYLOS_VERSION_TOKENS: typ.Final = ("4.33.2",)
_SKYLOS_PRODUCTION_TARGET_TOKENS: typ.Final = ("femtologging",)
_SKYLOS_EXCLUSION_TOKENS: typ.Final = (
    "femtologging/unittests",
    "femtologging/_femtologging_rs.pyi",
)
_SKYLOS_CLI_TOKENS: typ.Final = (
    "$(UV_ENV)",
    "uv",
    "tool",
    "run",
    "--python",
    "3.14",
    "--from",
    "skylos==$(SKYLOS_VERSION)",
    "skylos",
)
_SKYLOS_SCAN_TOKENS: typ.Final = (
    "$(SKYLOS_CLI)",
    "--config-file",
    "pyproject.toml",
)
_SKYLOS_LINT_TOKENS: typ.Final = (
    "$(SKYLOS)",
    "$(SKYLOS_PRODUCTION_TARGETS)",
    "$(SKYLOS_EXCLUDE_FLAGS)",
    "--category",
    "dead_code",
    "--gate",
    "--format",
    "concise",
    "--no-upload",
    "--no-provenance",
    "--no-grep-verify",
)
_SKYLOS_WHITELIST_TOKENS: typ.Final = (
    "flock",
    "$(SKYLOS_WHITELIST_LOCK)",
    "env",
    "$(SKYLOS_CLI)",
    "whitelist",
    "$${SKYLOS_SYMBOL}",
    "--reason",
    "$${SKYLOS_REASON}",
)
_SKYLOS_WHITELIST_LOCK_TOKENS: typ.Final = (".skylos-whitelist.lock",)
_DOCUMENTED_WHITELIST_NAMES: typ.Final[frozenset[str]] = frozenset()
_RUNTIME_IMPORT_ENTRY_POINTS: typ.Final = frozenset({
    "femtologging._femtologging_rs",
})
_RUNTIME_VARIABLE_ENTRY_POINTS: typ.Final = frozenset({
    "femtologging._rust_compat._runtime_attachment_state_for_test",
})
_RUNTIME_PARAMETER_ENTRY_POINTS: typ.Final = frozenset({
    "femtologging._rust_compat._make_rotating_fresh_failure_hooks._force.count",
    "femtologging._rust_compat._make_rotating_fresh_failure_hooks._force.reason",
    "femtologging._rust_compat._make_timed_rotation_hooks._set.epoch_millis",
    "femtologging._rust_compat._make_runtime_attachment_state._fallback.name",
})
_ENTRY_POINT_TYPES: typ.Final = (
    ("import", _RUNTIME_IMPORT_ENTRY_POINTS),
    ("variable", _RUNTIME_VARIABLE_ENTRY_POINTS),
    ("parameter", _RUNTIME_PARAMETER_ENTRY_POINTS),
)
# Every full-suite job must provision the pinned Makefile parser itself.
_MAKEUTIL_WORKFLOW_JOBS: typ.Final = (
    (".github/workflows/ci.yml", "build-test"),
    (".github/workflows/heavy-tests.yml", "heavy"),
)


def _skylos_configuration() -> dict[str, object]:
    """Load the repository's Skylos configuration."""
    with (repo_root() / "pyproject.toml").open("rb") as configuration_file:
        configuration = tomllib.load(configuration_file)
    tool = mapping(configuration.get("tool"), subject="tool configuration")
    return mapping(tool.get("skylos"), subject="Skylos configuration")


def _documented_whitelist_names(skylos: dict[str, object]) -> frozenset[str]:
    """Return the documented Skylos whitelist names, if configured."""
    whitelist = mapping(skylos.get("whitelist", {}), subject="Skylos whitelist")
    documented = mapping(
        whitelist.get("documented", {}), subject="Skylos whitelist entries"
    )
    return frozenset(documented)


def _assert_makeutil_installation(command: object, *, contract: str) -> None:
    """Assert that `command` installs the pinned Makeutil parser."""
    assert isinstance(command, str), (
        f"{contract} must provide a Makeutil installation shell command"
    )
    tokens = tuple(shlex.split(command.replace("\\\n", "")))
    assert tokens == _MAKEUTIL_INSTALL_TOKENS, (
        f"{contract} must pin the Makeutil installation command"
    )


def test_lint_recipe_runs_the_production_dead_code_gate() -> None:
    """`make lint` must scan production code with Skylos's strict gate."""
    test_prerequisites = text_sequence(
        sole_recipe_rule("test").get("prerequisites"),
        subject="test target prerequisites",
    )
    assert "makeutil" in test_prerequisites, (
        "Make test prerequisite contract must require makeutil"
    )
    assert variable_tokens("SKYLOS_VERSION") == _SKYLOS_VERSION_TOKENS, (
        "Skylos version contract must pin the reviewed release"
    )
    assert (
        variable_tokens("SKYLOS_PRODUCTION_TARGETS") == _SKYLOS_PRODUCTION_TARGET_TOKENS
    ), "Skylos production-target contract must scan the femtologging package"
    assert variable_tokens("SKYLOS_EXCLUDE_FOLDERS") == _SKYLOS_EXCLUSION_TOKENS, (
        "Skylos exclusion contract must omit unit tests and the native stub"
    )
    lint_prerequisites = text_sequence(
        sole_recipe_rule("lint", require_recipes=False).get("prerequisites"),
        subject="lint target prerequisites",
    )
    assert lint_prerequisites == ("lint-python", "lint-rust"), (
        "Skylos lint delegation contract must retain the Python lint target"
    )
    skylos_commands = [
        command
        for command in recipe_tokens("lint-python")
        if command[:1] == _SKYLOS_LINT_TOKENS[:1]
    ]

    assert skylos_commands == [_SKYLOS_LINT_TOKENS], (
        "Skylos lint command contract must scan production dead code strictly"
    )


def test_whitelist_target_uses_skylos_subcommand_contract() -> None:
    """`skylos whitelist` must precede the name and have no scan options."""
    assert variable_tokens("SKYLOS_CLI") == _SKYLOS_CLI_TOKENS, (
        "Skylos CLI contract must pin Python 3.14 and its tool release"
    )
    assert variable_tokens("SKYLOS") == _SKYLOS_SCAN_TOKENS, (
        "Skylos scan command contract must add only the configuration file"
    )
    assert variable_tokens("SKYLOS_WHITELIST_LOCK") == _SKYLOS_WHITELIST_LOCK_TOKENS, (
        "Skylos whitelist contract must use a repository-local lock"
    )

    whitelist_commands = [
        command
        for command in recipe_tokens("skylos-allow")
        if command[:4] == _SKYLOS_WHITELIST_TOKENS[:4]
    ]
    assert whitelist_commands == [_SKYLOS_WHITELIST_TOKENS], (
        "Skylos whitelist command contract must lock and dispatch before --reason"
    )


def test_skylos_configuration_models_implicit_runtime_callers() -> None:
    """Each current false positive must be a typed, explained entry point."""
    skylos = _skylos_configuration()
    gate = mapping(skylos.get("gate"), subject="Skylos gate configuration")
    assert gate.get("strict") is True, (
        "Skylos gate configuration must enable strict mode"
    )
    assert _documented_whitelist_names(skylos) == _DOCUMENTED_WHITELIST_NAMES, (
        "Skylos documented-whitelist contract must preserve reviewed exceptions"
    )
    dead_code = mapping(
        skylos.get("dead_code"), subject="Skylos dead-code configuration"
    )
    entry_points = objects(dead_code.get("entrypoints"), subject="Skylos entry points")

    entry_point_names = frozenset(
        name
        for entry_point in entry_points
        for name in text_sequence(
            entry_point.get("full_name"), subject="entry-point name"
        )
    )
    expected_names = frozenset().union(*(names for _, names in _ENTRY_POINT_TYPES))
    assert entry_point_names == expected_names, (
        "Skylos entry-point contract must preserve every runtime caller exclusion"
    )
    for entry_point in entry_points:
        names = frozenset(
            text_sequence(entry_point.get("full_name"), subject="entry-point name")
        )
        expected_types = [
            entry_type for entry_type, members in _ENTRY_POINT_TYPES if names & members
        ]
        assert len(expected_types) == 1, (
            "Skylos entry-point contract must not mix definition kinds in one rule"
        )
        assert entry_point.get("type") == expected_types[0], (
            "Skylos entry-point contract must classify each implicit runtime caller"
        )
        reason = entry_point.get("reason")
        assert isinstance(reason, str), (
            "Skylos entry-point contract must provide a textual reason"
        )
        assert reason, "Skylos entry-point contract must provide a non-empty reason"


def test_ci_runs_the_lint_target_and_installs_makeutil() -> None:
    """CI must run the same lint target and provide its Makefile parser."""
    lint_step = sole_workflow_step(".github/workflows/ci.yml", "build-test", "Lint")
    assert lint_step.get("run") == "make lint", (
        "CI lint-step contract must invoke the shared make lint target"
    )

    for workflow_path, job_name in _MAKEUTIL_WORKFLOW_JOBS:
        job = workflow_job(workflow_path, job_name)
        environment = mapping(
            job.get("env"), subject=f"{workflow_path} Makeutil environment"
        )
        assert environment.get("MAKEUTIL_REVISION") == _MAKEUTIL_REVISION, (
            f"{workflow_path} {job_name} Makeutil revision contract must stay pinned"
        )
        assert environment.get("MAKEUTIL_TOOLCHAIN") == _MAKEUTIL_TOOLCHAIN, (
            f"{workflow_path} {job_name} Makeutil toolchain contract must stay pinned"
        )
        parser_step = sole_workflow_step(
            workflow_path, job_name, "Install Makefile parser"
        )
        _assert_makeutil_installation(
            parser_step.get("run"),
            contract=f"{workflow_path} {job_name} Makeutil-install contract",
        )
