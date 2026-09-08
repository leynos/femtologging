"""Contracts for Makefile targets outside the lint-tier pipeline."""

from __future__ import annotations

from tests.make_contract_helpers import (
    recipe_tokens,
    sole_recipe_rule,
    text_sequence,
)

type Tokens = tuple[str, ...]

_SPELLING_SOURCES: Tokens = (
    "scripts/generate_typos_config.py",
    "scripts/typos_rollout.py",
    "scripts/typos_rollout_cache.py",
    "scripts/tests",
)
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
_SPELLING_COMMANDS: tuple[Tokens, ...] = (
    ("@$(UV_ENV)", "uv", "run", "scripts/generate_typos_config.py"),
    (
        "@git",
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
        "|",
        "xargs",
        "-0",
        "-r",
        "env",
        "$(UV_ENV)",
        "uv",
        "tool",
        "run",
        "typos@$(TYPOS_VERSION)",
        "--config",
        "typos.toml",
        "--force-exclude",
    ),
)
_SPELLING_HELPER_COMMANDS: tuple[Tokens, ...] = (
    (
        "@$(UV_ENV)",
        "uv",
        "tool",
        "run",
        "ruff@$(RUFF_VERSION)",
        "format",
        "--isolated",
        "--target-version",
        "py313",
        "--check",
        *_SPELLING_SOURCES,
    ),
    (
        "@$(UV_ENV)",
        "uv",
        "tool",
        "run",
        "ruff@$(RUFF_VERSION)",
        "check",
        "--isolated",
        "--target-version",
        "py313",
        *_SPELLING_SOURCES,
    ),
    (
        "@PYTHONPATH=scripts",
        "$(UV_ENV)",
        "uv",
        "run",
        "--no-project",
        "--python",
        "3.13",
        "--with",
        "pytest==9.0.2",
        "--with",
        "pytest-cov==7.0.0",
        "python",
        "-m",
        "pytest",
        "scripts/tests",
        "-c",
        "/dev/null",
        "--rootdir=.",
        "-p",
        "no:cacheprovider",
        "--cov=generate_typos_config",
        "--cov=typos_rollout",
        "--cov=typos_rollout_cache",
        "--cov-fail-under=90",
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


def test_typecheck_recipe_uses_the_project_extension_environment() -> None:
    """`typecheck` must direct the pinned ty tool at the built project venv."""
    assert _prerequisites("typecheck") == ("build",), (
        "typecheck must build the extension before static analysis"
    )
    assert _commands("typecheck") == (
        ("$(TY)", "check", "--python", ".venv", "--extra-search-path", "scripts"),
    ), "typecheck must preserve its complete ty invocation"


def test_markdownlint_recipe_requires_spelling_and_scans_repository_markdown() -> None:
    """`markdownlint` must run spelling first and scan only repository Markdown."""
    assert _prerequisites("markdownlint") == ("spelling",), (
        "markdownlint must enforce spelling before Markdown structure"
    )
    assert _commands("markdownlint") == ((*_MARKDOWN_FILES, "$(MDLINT)", "--"),), (
        "markdownlint must lint tracked and non-ignored Markdown files only"
    )


def test_spelling_recipe_generates_policy_and_runs_the_pinned_checker() -> None:
    """`spelling` must validate helpers, regenerate policy, and run pinned typos."""
    assert _prerequisites("spelling") == ("spelling-helper-test",), (
        "spelling must validate policy helpers before regenerating typos.toml"
    )
    assert _commands("spelling") == _SPELLING_COMMANDS, (
        "spelling must check tracked and non-ignored source and prose with pinned typos"
    )


def test_spelling_helper_recipe_checks_format_lint_and_coverage() -> None:
    """`spelling-helper-test` must validate its helpers before policy generation."""
    assert _prerequisites("spelling-helper-test") == (), (
        "spelling helper validation must not depend on generated policy output"
    )
    assert _commands("spelling-helper-test") == _SPELLING_HELPER_COMMANDS, (
        "spelling helper validation must retain all three complete commands"
    )
