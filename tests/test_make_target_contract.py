"""Contracts for Makefile targets outside the lint-tier pipeline."""

from __future__ import annotations

import tomllib
import typing as typ

from tests.make_contract_helpers import (
    recipe_tokens,
    repo_root,
    sole_recipe_rule,
    text_sequence,
    variable_tokens,
)

if typ.TYPE_CHECKING:
    from syrupy.assertion import SnapshotAssertion

type Tokens = tuple[str, ...]

_MARKDOWN_FILES: Tokens = (
    "git",
    "ls-files",
    "-z",
    "--cached",
    "--others",
    "--exclude-standard",
    "*.md",
    "|",
    "xargs",
    "-0",
    "-r",
)

_UNTRACKED_MARKDOWN_FILES: Tokens = (
    "git",
    "ls-files",
    "-z",
    "--others",
    "--exclude-standard",
    "*.md",
    "|",
    "xargs",
    "-0",
    "-r",
)

#: The three commands `spelling` must run, in order: the builder gate, the
#: same pinned release's Typos binary over the Markdown the gate cannot see,
#: and the gate's phrase check over the same files through a throwaway index.
#: The phrase check reads an index, and Typos cannot express the shared phrase
#: corrections, so an intent-to-add entry is the only way to show it a file
#: that is not staged without disturbing the real index.
_SPELLING_COMMANDS: tuple[Tokens, ...] = (
    ("@$(UV_ENV)", "$(TYPOS_CONFIG_BUILDER)", "gate"),
    (
        *_UNTRACKED_MARKDOWN_FILES,
        "env",
        "$(UV_ENV)",
        "$(TYPOS)",
        "--config",
        "typos.toml",
        "--force-exclude",
        "--",
    ),
    (
        "@spelling_index=$$(mktemp)",
        "&&",
        "trap",
        'rm -f "$$spelling_index"',
        "EXIT;",
        "GIT_INDEX_FILE=$$spelling_index",
        "git",
        "read-tree",
        "--empty",
        "&&",
        "{",
        "git",
        "ls-files",
        "-z",
        "--others",
        "--exclude-standard",
        "*.md",
        "|",
        "xargs",
        "-0",
        "-r",
        "env",
        "GIT_INDEX_FILE=$$spelling_index",
        "git",
        "add",
        "--intent-to-add",
        "--",
        ";",
        "}",
        "&&",
        "$(UV_ENV)",
        "GIT_INDEX_FILE=$$spelling_index",
        "$(SPELLING_PHRASE_SCAN)",
    ),
)

#: The two commands `spelling-integration-test` must run, in order.
_SPELLING_INTEGRATION_COMMANDS: tuple[Tokens, ...] = (
    ("@$(UV_ENV)", "$(TYPOS_CONFIG_BUILDER)", "--help", ">", "/dev/null"),
    (
        "@$(SPELLING_INTEGRATION_ENV)",
        "PYTHONPATH=.",
        "$(UV_ENV)",
        "uv",
        "run",
        "--no-project",
        "--python",
        "3.13",
        "--with",
        "pytest==9.0.2",
        "--with",
        "pytest-timeout==2.4.0",
        "--with",
        "pyyaml",
        "python",
        "-m",
        "pytest",
        "tests/test_spelling_gate_end_to_end.py",
        "-c",
        "/dev/null",
        "--rootdir=.",
        "--noconftest",
        "-p",
        "no:cacheprovider",
    ),
)


_CHECK_API_BOUNDARY_COMMANDS: tuple[Tokens, ...] = (
    (
        "$(CARGO_BUILD_ENV)",
        "UV_CONFIG_FILE=$(CURDIR)/.github/uv-test-util.toml",
        "uv",
        "run",
        "pytest",
        "-v",
        "-o",
        "addopts=",
        "tests/test_release_api_boundary.py",
    ),
)


def _commands(target: str) -> tuple[Tokens, ...]:
    """Return non-comment recipe commands parsed for *target*."""
    return tuple(
        tuple(token for token in command if token.strip())
        for command in recipe_tokens(target)
        if command
    )


def _prerequisites(target: str) -> Tokens:
    """Return parsed prerequisites for the Makefile *target*."""
    rule = sole_recipe_rule(target)
    return text_sequence(rule.get("prerequisites"), subject=f"{target} prerequisites")


def test_build_recipe_installs_from_the_committed_lock(
    snapshot: SnapshotAssertion,
) -> None:
    """`build` syncs the dev group and runs maturin in locked mode."""
    assert _commands("build") == snapshot, (
        "build must resolve dependencies from uv.lock and refuse drift"
    )


def test_test_recipe_runs_pytest_in_locked_mode(
    snapshot: SnapshotAssertion,
) -> None:
    """`test` runs pytest through uv in locked mode."""
    pytest_commands = [command for command in _commands("test") if "pytest" in command]
    assert pytest_commands == snapshot, (
        "the pytest run must refuse a lockfile that diverges from the manifest"
    )


def test_typecheck_recipe_uses_the_project_extension_environment() -> None:
    """`typecheck` must direct the pinned ty tool at the built project venv."""
    assert _prerequisites("typecheck") == ("check-build-tools", "build"), (
        "typecheck must check the build tools, then build the extension, "
        "before static analysis"
    )
    assert _commands("typecheck") == (("$(TY)", "check", "--python", ".venv"),), (
        "typecheck must preserve its complete ty invocation"
    )


def test_check_api_boundary_builds_then_runs_the_boundary_suite() -> None:
    """`check-api-boundary` must build the extension before probing packaging."""
    assert _prerequisites("check-api-boundary") == ("build",), (
        "the boundary check needs the development environment that `build` makes"
    )
    assert _commands("check-api-boundary") == _CHECK_API_BOUNDARY_COMMANDS, (
        "the boundary check must run its dedicated pytest module"
    )


def test_default_collection_skips_the_api_boundary_suite() -> None:
    """Only `check-api-boundary` may collect the release-wheel boundary suite."""
    pyproject = repo_root() / "pyproject.toml"
    project = tomllib.loads(pyproject.read_text(encoding="utf-8"))
    addopts = project["tool"]["pytest"]["ini_options"]["addopts"]
    assert "--ignore=tests/test_release_api_boundary.py" in addopts, (
        "default pytest collection must leave the release-wheel build to "
        "`make check-api-boundary`"
    )


def test_markdownlint_recipe_requires_spelling_and_scans_repository_markdown() -> None:
    """`markdownlint` must run spelling first and scan only repository Markdown."""
    assert _prerequisites("markdownlint") == ("spelling",), (
        "markdownlint must enforce spelling before Markdown structure"
    )
    assert _commands("markdownlint") == ((*_MARKDOWN_FILES, "$(MDLINT)", "--"),), (
        "markdownlint must lint tracked and non-ignored Markdown files only"
    )


def test_spelling_recipe_runs_the_pinned_config_builder_gate() -> None:
    """`spelling` must gate the index, then check unstaged files and phrases."""
    assert variable_tokens("TYPOS_CONFIG_BUILDER") == (
        "uvx",
        "--python",
        "3.14",
        "--from",
        "git+https://github.com/leynos/typos-config-builder.git@v0.1.3",
        "typos-config-builder",
    ), (
        "the builder must stay pinned to the reviewed release, on an "
        "interpreter it accepts: it declares Python >= 3.14, while a hosted "
        "suite job exports UV_PYTHON for its own lane"
    )
    assert variable_tokens("TYPOS") == (
        "uvx",
        "--quiet",
        "--python",
        "3.14",
        "--from",
        "git+https://github.com/leynos/typos-config-builder.git@v0.1.3",
        "typos",
    ), (
        "the unstaged-file stage must run the Typos binary the builder pins, "
        "from the same release, so both stages apply identical spelling rules"
    )
    assert variable_tokens("SPELLING_PHRASE_SCAN") == (
        "$(TYPOS_CONFIG_BUILDER)",
        "check-phrases",
    ), (
        "the phrase stage must ask the builder's own check, so the forbidden "
        "phrases and their corrections come from the same shared table"
    )
    assert not _prerequisites("spelling"), (
        "the gate owns dictionary refresh, generation and checking"
    )
    assert _commands("spelling") == _SPELLING_COMMANDS, (
        "spelling must run the builder gate, then check the Markdown the gate "
        "cannot see: new files that are not yet staged, with their file policy "
        "still in force, and finally the phrase check over those files"
    )


def test_spelling_config_target_is_the_only_write_path() -> None:
    """`spelling-config` regenerates the file; `spelling-check` must not."""
    assert _commands("spelling-config") == (
        ("@$(UV_ENV)", "$(TYPOS_CONFIG_BUILDER)", "gate"),
    ), "the write path must be the builder gate, and nothing else"
    assert _commands("spelling-check") == (
        ("@$(UV_ENV)", "$(TYPOS_CONFIG_BUILDER)", "--check", "gate"),
    ), (
        "the read-only path must pass the builder-wide --check option before "
        "the gate subcommand"
    )
    assert not _prerequisites("spelling-config"), (
        "regeneration must not depend on a check that would run first"
    )
    assert not _prerequisites("spelling-check"), (
        "the drift check must not regenerate anything on the way in"
    )


def test_the_phony_list_declares_the_new_spelling_targets() -> None:
    """Both new targets must be phony, or a file of the same name shadows them."""
    lines = (repo_root() / "Makefile").read_text("utf-8").splitlines()
    start = next(i for i, line in enumerate(lines) if line.startswith(".PHONY:"))
    # The list is continued across lines; collect it until a line without the
    # continuation backslash.
    parts: list[str] = []
    for line in lines[start:]:
        parts.append(line)
        if not line.rstrip().endswith("\\"):
            break
    phony = " ".join(parts)
    assert "spelling-config" in phony, phony
    assert "spelling-check" in phony, phony


def test_spelling_integration_target_provisions_the_pinned_builder() -> None:
    """The end-to-end target warms the pin and sets its integration flag."""
    assert (
        _commands("spelling-integration-test")[:2] == _SPELLING_INTEGRATION_COMMANDS
    ), "the integration target must provision the pin, then run its tests"
