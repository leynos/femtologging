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


def _recipe_text() -> str:
    """Return the target's recipes joined into one searchable string."""
    return "\n".join(" ".join(tokens) for tokens in recipe_tokens(_TARGET))


def test_rustdoc_denies_warnings() -> None:
    """A rustdoc run that tolerates warnings is not a gate."""
    recipe = _recipe_text()
    sets_flags = "RUSTDOCFLAGS" in recipe
    denies_warnings = "-D warnings" in recipe
    assert sets_flags, f"the rustdoc target must set RUSTDOCFLAGS, got {recipe}"
    assert denies_warnings, f"the rustdoc target must deny warnings, got {recipe}"


def test_rustdoc_stays_within_this_crate() -> None:
    """`--no-deps` keeps the gate to this crate's own documentation."""
    args = variable_tokens("RUST_DOC_ARGS")
    assert "--no-deps" in args, (
        "RUST_DOC_ARGS must carry --no-deps so dependency documentation is not "
        f"built or gated, got {args}"
    )


def test_lint_rust_depends_on_the_rustdoc_gate() -> None:
    """The aggregate must not be able to skip the documentation lane."""
    rule = sole_recipe_rule("lint-rust")
    prerequisites = text_sequence(rule.get("prerequisites"), subject="prerequisites")
    assert _TARGET in prerequisites, (
        "lint-rust must depend on lint-rust-doc, because `make lint` is the only "
        f"aggregate CI invokes, got {prerequisites}"
    )


def test_the_lane_list_covers_the_documentation_surface() -> None:
    """Each documented feature selection needs its own rustdoc lane."""
    lanes = variable_tokens("RUST_DOC_FEATURE_LANES")
    for expected in ("none", "python", "log-compat", "tracing-compat", "all"):
        assert expected in lanes, (
            f"{expected!r} must be a declared rustdoc lane, got {lanes}"
        )
