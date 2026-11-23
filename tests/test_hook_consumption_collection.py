"""Collection contract for test modules that use the gated test hooks.

A production extension does not define the `*_for_test` wrappers. The modules
that consume them must still be importable, because pytest has to collect them
before it can apply a skip marker; an import error at collection is not a skip.

The probe hides the hooks from a `test-util` build and collects the consumers,
which reproduces a hook-absent build without building a second extension.
"""

from __future__ import annotations

import os
import subprocess  # ruff: ignore[suspicious-subprocess-import] the probe runs a fresh interpreter.
import sys
import typing as typ

if typ.TYPE_CHECKING:
    from pathlib import Path

from tests.make_contract_helpers import repo_root

_HOOK_HIDING_PLUGIN = """\
import femtologging._rust_compat as compat

_HOOKS = (
    "_force_rotating_fresh_failure_for_test",
    "_clear_rotating_fresh_failure_for_test",
    "_set_timed_rotation_test_times_for_test",
    "_clear_timed_rotation_test_times_for_test",
    "_runtime_attachment_state_for_test",
)


def pytest_configure(config):
    del config
    for name in _HOOKS:
        if hasattr(compat, name):
            delattr(compat, name)
"""

_HOOK_CONSUMERS = (
    "tests/test_dict_config.py",
    "tests/steps/test_rotating_rotation_steps.py",
    "tests/steps/test_timed_rotating_rotation_steps.py",
)


def test_hook_consumers_collect_without_the_test_hooks(tmp_path: Path) -> None:
    """A build without test-util still collects every hook-consuming module."""
    plugin = tmp_path / "hide_test_hooks.py"
    plugin.write_text(_HOOK_HIDING_PLUGIN, encoding="utf-8")
    # A hook-absent build is not a promised test-util build, so drop the
    # variable that makes conftest reject it before collection.
    environment = {
        key: value
        for key, value in os.environ.items()
        if key != "FEMTOLOGGING_EXPECT_TEST_UTIL"
    }
    environment["PYTHONPATH"] = str(tmp_path)
    # ruff: ignore[subprocess-without-shell-equals-true] fixed probe of the
    # running interpreter and repository-local test modules.
    result = subprocess.run(
        [
            sys.executable,
            "-m",
            "pytest",
            "--collect-only",
            "-q",
            "-p",
            "hide_test_hooks",
            *_HOOK_CONSUMERS,
        ],
        check=False,
        capture_output=True,
        cwd=repo_root(),
        env=environment,
        text=True,
        timeout=300,
    )
    assert result.returncode == 0, (
        "the hook-consuming test modules must collect without the test hooks:\n"
        f"{result.stdout}{result.stderr}"
    )
