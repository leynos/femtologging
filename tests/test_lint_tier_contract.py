"""Contract tests for the Python lint gate's tiers in the Makefile.

`make lint` delegates to `lint-python`, which must run every tier in order:
Ruff, the `interrogate` docstring-coverage gate, PyPy-backed Pylint, the
df12-python-lints house rules, the `ambrleaks` snapshot scanner, and the
Skylos dead-code gate. Each tier's runtime and pinned
revision is load-bearing — the df12 checkers and Skylos deliberately run on a
newer interpreter than the project's 3.12 baseline — so this module pins the
whole recipe shape rather than trusting that a tier is still wired in.

The Skylos command itself is covered in detail by
`tests/test_skylos_lint_contract.py`; here it is only asserted to be present
and last, so the two modules do not duplicate one another.
"""

from __future__ import annotations

import typing as typ

import pytest

from tests.make_contract_helpers import recipe_tokens, variable_tokens

# The tier commands, in the order `lint-python` must run them.
_LINT_TIER_COMMANDS: typ.Final = (
    ("$(RUFF)", "check"),
    ("$(INTERROGATE)", "$(INTERROGATE_TARGETS)"),
    ("$(PYLINT)", "$(PYLINT_TARGETS)"),
    ("$(DF12_PYLINT)", "$(PYLINT_TARGETS)"),
    ("$(AMBRLEAKS)", "tests", "femtologging/unittests"),
)
_SKYLOS_COMMAND_HEAD: typ.Final = ("$(SKYLOS)", "$(SKYLOS_PRODUCTION_TARGETS)")

_RUFF_TOKENS: typ.Final = ("uvx", "ruff==$(RUFF_VERSION)")
_TY_TOKENS: typ.Final = (
    "$(UV_ENV)",
    "uv",
    "tool",
    "run",
    "--from",
    "ty==$(TY_VERSION)",
    "ty",
)
_PYLINT_TOKENS: typ.Final = (
    "$(UV_ENV)",
    "uv",
    "tool",
    "run",
    "--python",
    "$(PYLINT_PYTHON)",
    "--from",
    "$(PYLINT_PYPY_SHIM)",
    "--with",
    "pylint==$(PYLINT_VERSION)",
    "pylint-pypy",
)
_DF12_PYLINT_TOKENS: typ.Final = (
    "$(UV_ENV)",
    "uv",
    "tool",
    "run",
    "--python",
    "$(DF12_PYTHON)",
    "--from",
    "pylint==$(PYLINT_VERSION)",
    "--with",
    "$(DF12_PYTHON_LINTS)",
    "pylint",
    "--disable=all",
    "--load-plugins=df12_python_lints",
    "--enable=$(DF12_PYLINT_MESSAGES)",
)
# Docstring coverage is enforced at 100% over the production package only.
_INTERROGATE_TOKENS: typ.Final = (
    "$(UV_ENV)",
    "uv",
    "tool",
    "run",
    "--from",
    "interrogate==$(INTERROGATE_VERSION)",
    "interrogate",
    "--fail-under",
    "100",
    "--ignore-regex",
    "$(INTERROGATE_IGNORE_REGEX)",
)
_INTERROGATE_TARGET_TOKENS: typ.Final = ("femtologging",)
# Only the overload stubs are exempt; Ruff still requires the implementation's
# docstring, so this must not widen into a general escape hatch.
_INTERROGATE_IGNORE_REGEX_TOKENS: typ.Final = ("^basicConfig$$",)
_AMBRLEAKS_TOKENS: typ.Final = (
    "$(UV_ENV)",
    "uv",
    "tool",
    "run",
    "--python",
    "$(DF12_PYTHON)",
    "--from",
    "$(DF12_PYTHON_LINTS)",
    "ambrleaks",
)
_PYLINT_TARGET_TOKENS: typ.Final = ("femtologging", "tests", "scripts")
_DF12_PYLINT_MESSAGE_TOKENS: typ.Final = (
    "R9101,C9102,R9103,R9104,C9105,C9106,C9107,R9108,R9109,R9110,R9111,R9112,C9112",
)
# The df12 checkers and Skylos parse source with their own runtime AST, so they
# must lead the project's 3.12 syntax baseline rather than match it.
_DF12_PYTHON_TOKENS: typ.Final = ("3.14",)
_PYLINT_PYTHON_TOKENS: typ.Final = ("pypy",)

# Pins with no CI counterpart to sync against; assert they stay pinned at all.
_PINNED_REVISIONS: typ.Final = (
    ("PYLINT_VERSION", "4.0.7"),
    ("PYLINT_PYPY_SHIM_REF", "726d09f968b4d729ee4b29c71fc732e744854f3b"),
    ("DF12_PYTHON_LINTS_REF", "4cf41736cce2f7ba2778882a5c629c044568a0e5"),
    ("INTERROGATE_VERSION", "1.7.0"),
)


def test_lint_python_runs_every_tier_in_order() -> None:
    """`lint-python` must run every lint tier, Skylos last."""
    commands = recipe_tokens("lint-python")

    assert len(commands) == len(_LINT_TIER_COMMANDS) + 1, (
        "Lint tier contract must run exactly the five tiers plus the Skylos gate"
    )
    assert commands[: len(_LINT_TIER_COMMANDS)] == _LINT_TIER_COMMANDS, (
        "Lint tier contract must run Ruff, interrogate, Pylint, "
        "df12-python-lints, and ambrleaks in that order"
    )
    assert commands[-1][: len(_SKYLOS_COMMAND_HEAD)] == _SKYLOS_COMMAND_HEAD, (
        "Lint tier contract must finish with the Skylos production dead-code gate"
    )


@pytest.mark.parametrize(
    ("variable_name", "expected_tokens"),
    [
        ("RUFF", _RUFF_TOKENS),
        ("TY", _TY_TOKENS),
        ("INTERROGATE", _INTERROGATE_TOKENS),
        ("PYLINT", _PYLINT_TOKENS),
        ("DF12_PYLINT", _DF12_PYLINT_TOKENS),
        ("AMBRLEAKS", _AMBRLEAKS_TOKENS),
    ],
)
def test_tier_commands_consume_their_pinned_versions(
    variable_name: str, expected_tokens: tuple[str, ...]
) -> None:
    """Each tier command must interpolate its pinned version variable."""
    assert variable_tokens(variable_name) == expected_tokens, (
        f"Lint tier contract must keep {variable_name} interpolating its "
        f"pinned version and runtime"
    )


@pytest.mark.parametrize(
    ("variable_name", "expected_tokens"),
    [
        ("PYLINT_TARGETS", _PYLINT_TARGET_TOKENS),
        ("INTERROGATE_TARGETS", _INTERROGATE_TARGET_TOKENS),
        ("INTERROGATE_IGNORE_REGEX", _INTERROGATE_IGNORE_REGEX_TOKENS),
        ("DF12_PYLINT_MESSAGES", _DF12_PYLINT_MESSAGE_TOKENS),
        ("DF12_PYTHON", _DF12_PYTHON_TOKENS),
        ("PYLINT_PYTHON", _PYLINT_PYTHON_TOKENS),
    ],
)
def test_tier_configuration_variables_stay_pinned(
    variable_name: str, expected_tokens: tuple[str, ...]
) -> None:
    """Tier targets, message set, and runtimes must not drift silently."""
    assert variable_tokens(variable_name) == expected_tokens, (
        f"Lint tier contract must preserve {variable_name}"
    )


@pytest.mark.parametrize(("variable_name", "expected_revision"), _PINNED_REVISIONS)
def test_tier_revisions_are_pinned(variable_name: str, expected_revision: str) -> None:
    """Pylint and df12-python-lints must stay pinned to reviewed revisions."""
    assert variable_tokens(variable_name) == (expected_revision,), (
        f"Lint tier contract must pin {variable_name}; an unpinned tier would "
        f"change lint behaviour with no repository change"
    )
