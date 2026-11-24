"""Collection and run contract for test modules that use the gated test hooks.

A production extension does not define the `*_for_test` wrappers and reports
`_has_test_util` as `False`. The modules that consume those hooks must still be
importable, because pytest has to collect them before it can apply a skip
marker; an import error at collection is not a skip. The independent tests in
those modules must also still run.

The probe hides the hooks from a `test-util` build and runs the plain test
modules that consume them, which reproduces a hook-absent build without
building a second extension. BDD step modules need their whole feature's step
set and cannot run standalone; their identity steps call the same
`require_identity_hooks` guard, and the feature runs cover them.
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
import femtologging
import femtologging._rust_compat as compat

# The native hooks keep their unprefixed names; the compatibility wrappers add
# an underscore. Delete both spellings so the probe matches a production build.
_NATIVE_HOOKS = (
    "force_rotating_fresh_failure_for_test",
    "clear_rotating_fresh_failure_for_test",
    "set_timed_rotation_test_times_for_test",
    "clear_timed_rotation_test_times_for_test",
    "runtime_attachment_state_for_test",
    "_rust_log_enabled",
    "_rust_log_max_level",
    "_emit_rust_log",
    "_install_test_global_rust_logger",
)


def pytest_configure(config):
    del config
    femtologging._has_test_util = False
    for name in _NATIVE_HOOKS:
        if hasattr(compat, f"_{name}"):
            delattr(compat, f"_{name}")
        if hasattr(femtologging.rust, name):
            delattr(femtologging.rust, name)
"""

_HOOK_CONSUMERS = (
    "tests/test_basic_config.py",
    "tests/test_config_transaction_property.py",
    "tests/test_dict_config.py",
    "tests/test_disable_existing_loggers.py",
    "tests/test_runtime_definition_lifecycle.py",
    "tests/test_runtime_reconfiguration.py",
)


def test_hook_consumers_run_without_the_test_hooks(tmp_path: Path) -> None:
    """A build without test-util collects and runs every hook-consuming module."""
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
        "the hook-consuming test modules must collect and run without the test "
        f"hooks, skipping only the hook-dependent cases:\n"
        f"{result.stdout}{result.stderr}"
    )
    assert "passed" in result.stdout, "the independent tests must still run"
    assert "skipped" in result.stdout, "the hook-dependent tests must skip"
