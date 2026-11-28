"""Contract tests for the Rust documentation gate (issue #421).

The gate exists to keep broken intra-doc links out of the crate, and it only
does that while three things stay true: the target denies rustdoc warnings,
`lint-rust` depends on it, and the lane list covers the documentation surface.
Any of those can be removed in one line without any Rust test noticing, so they
are pinned here instead.
"""

from __future__ import annotations

import pytest

from tests.make_contract_helpers import (
    recipe_tokens,
    sole_recipe_rule,
    text_sequence,
    variable_tokens,
)

pytestmark = pytest.mark.timeout(120)

_TARGET = "lint-rust-doc"


# `\n` is a line continuation inside one command, not a separator.
_SEPARATORS = {";", "then", "elif", "else", "do", "fi;", "done"}


def _doc_invocations() -> list[list[str]]:
    """Return one token list per `cargo doc` command in the target.

    The recipe is a single shell script, so it is split into commands at shell
    separators. Each command that runs `cargo doc` is returned with its own
    environment prefix, because `RUSTDOCFLAGS` is set on the command rather than
    for the whole script — an assertion that looked anywhere in the recipe would
    pass even if one invocation lost the flag.

    Returns
    -------
        One token list per command that runs `cargo doc`.
    """
    commands: list[list[str]] = []
    current: list[str] = []
    for tokens in recipe_tokens(_TARGET):
        for token in tokens:
            if token in _SEPARATORS:
                commands.append(current)
                current = []
            else:
                current.append(token)
    commands.append(current)
    return [
        command for command in commands if "$(CARGO)" in command and "doc" in command
    ]


def test_every_invocation_denies_warnings() -> None:
    """A rustdoc run that tolerates warnings is not a gate."""
    invocations = _doc_invocations()
    assert invocations, "the target must invoke cargo doc at least once"
    for invocation in invocations:
        assert "doc" in invocation, f"expected a cargo doc call, got {invocation}"
        assert "RUSTDOCFLAGS=-D warnings" in invocation, (
            f"every cargo doc invocation must deny warnings, got {invocation}"
        )


def test_every_invocation_stays_within_this_crate() -> None:
    """`--no-deps` keeps the gate to this crate's own documentation."""
    args = variable_tokens("RUST_DOC_ARGS")
    assert "--no-deps" in args, (
        "RUST_DOC_ARGS must carry --no-deps so dependency documentation is not "
        f"built or gated, got {args}"
    )
    for invocation in _doc_invocations():
        assert "$(RUST_DOC_ARGS)" in invocation, (
            f"every cargo doc invocation must expand RUST_DOC_ARGS, got {invocation}"
        )


def test_lint_rust_depends_on_the_rustdoc_gate() -> None:
    """The aggregate must not be able to skip the documentation lane."""
    rule = sole_recipe_rule("lint-rust")
    prerequisites = text_sequence(rule.get("prerequisites"), subject="prerequisites")
    assert _TARGET in prerequisites, (
        "lint-rust must depend on lint-rust-doc, because `make lint` is the only "
        f"aggregate CI invokes, got {prerequisites}"
    )


def test_the_recipe_iterates_the_declared_lanes() -> None:
    """A declared lane list that the recipe ignores gates nothing."""
    script = " ".join(token for tokens in recipe_tokens(_TARGET) for token in tokens)
    assert "for features in $(RUST_DOC_FEATURE_LANES)" in script.replace("\n", " "), (
        "the recipe must loop over RUST_DOC_FEATURE_LANES"
    )
    assert "--no-default-features" in script, "the none lane must disable defaults"
    assert "--all-features" in script, "the all lane must enable every feature"


def test_the_lane_list_covers_the_documentation_surface() -> None:
    """Each documented feature selection needs its own rustdoc lane."""
    lanes = variable_tokens("RUST_DOC_FEATURE_LANES")
    for expected in ("none", "python", "log-compat", "tracing-compat", "all"):
        assert expected in lanes, (
            f"{expected!r} must be a declared rustdoc lane, got {lanes}"
        )
