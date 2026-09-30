"""Assertions for how a workflow provisions makeutil.

CI installs makeutil through the shared prebuilt ``install-makeutil`` action
and smoke-tests it straight away. These assertions hold both halves in one
place so the workflow contract that calls them stays small. They raise
``AssertionError`` directly because this module is a helper, not a test.
"""

from __future__ import annotations

import typing as typ

INSTALL_ACTION: typ.Final = (
    "leynos/shared-actions/.github/actions/install-makeutil"
    "@d57cb19b82281236088108f2ffb7e13bc00fc2f8"
)

_VERSION_CHECK: typ.Final = (
    'test "$(makeutil --version)" = "makeutil ${INSTALLED_VERSION}"'
)


def _require(condition: object, message: str) -> None:
    """Raise `AssertionError` with `message` unless `condition` holds."""
    if not condition:
        raise AssertionError(message)


def assert_installation(step: dict[str, object], *, contract: str) -> None:
    """Assert that `step` runs the pinned prebuilt install action, defaults only.

    A `run` key would mean a from-source install had crept back, and a `with`
    key would move the version off the action's own default and digest table.
    """
    _require(
        step.get("uses") == INSTALL_ACTION,
        f"{contract} must use the pinned install-makeutil action",
    )
    _require("run" not in step, f"{contract} must not also run an install command")
    _require(
        "with" not in step,
        f"{contract} must take the action's default version",
    )


def assert_verification(
    install_step: dict[str, object],
    verify_step: dict[str, object],
    *,
    contract: str,
) -> None:
    """Assert the smoke step that proves the installed binary is usable.

    The install step needs an id so the verify step can read the version the
    action reports; the verify step must compare the binary's own version with
    it and require a complete parse of the repository Makefile.
    """
    _require(install_step.get("id") == "makeutil", f"{contract} install step needs id")
    environment = verify_step.get("env")
    _require(isinstance(environment, dict), f"{contract} verify step needs an env")
    _require(
        isinstance(environment, dict)
        and environment.get("INSTALLED_VERSION")
        == "${{ steps.makeutil.outputs.version }}",
        f"{contract} must read the version the install action reports",
    )
    script = verify_step.get("run")
    _require(isinstance(script, str), f"{contract} must run a verification script")
    text = script if isinstance(script, str) else ""
    _require(
        _VERSION_CHECK in text,
        f"{contract} must compare the binary's version with the installed one",
    )
    _require(
        "makeutil parse Makefile" in text,
        f"{contract} must parse the repository Makefile",
    )
    _require(
        '["parse"]["status"] == "complete"' in text,
        f"{contract} must require a complete parse",
    )


def assert_verification_follows_install(
    steps: list[dict[str, object]], *, contract: str
) -> None:
    """Assert the verify step directly follows the install step."""
    names = [step.get("name") for step in steps]
    position = names.index("Install makeutil")
    _require(
        names[position + 1] == "Verify makeutil",
        f"{contract} must verify makeutil right after installing it",
    )
