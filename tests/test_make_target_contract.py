"""Contracts for Makefile targets outside the lint-tier pipeline."""

from __future__ import annotations

from tests.make_contract_helpers import (
    recipe_tokens,
    sole_recipe_rule,
    text_sequence,
    variable_tokens,
)

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
    assert _prerequisites("typecheck") == ("check-build-tools", "build"), (
        "typecheck must check the build tools, then build the extension, "
        "before static analysis"
    )
    assert _commands("typecheck") == (("$(TY)", "check", "--python", ".venv"),), (
        "typecheck must preserve its complete ty invocation"
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
    """`spelling` must delegate generation and checking to the builder gate."""
    assert variable_tokens("TYPOS_CONFIG_BUILDER") == (
        "uvx",
        "--from",
        "git+https://github.com/leynos/typos-config-builder.git@v0.1.3",
        "typos-config-builder",
    ), "the builder must stay pinned to the reviewed release"
    assert not _prerequisites("spelling"), (
        "the gate owns dictionary refresh, generation and checking"
    )
    assert _commands("spelling") == (
        ("@$(UV_ENV)", "$(TYPOS_CONFIG_BUILDER)", "gate"),
    ), "spelling must run the builder gate in write mode"
