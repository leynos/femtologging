"""Run re-entrant finalizer scenarios in a bounded child interpreter.

The scenarios under test need a fresh interpreter: a deadlocked finalizer would
hang the pytest process itself, and a process-wide logging manager reset in the
middle of collection teardown would confuse later tests. Running each fixture
script with a hard timeout makes a hang a test failure instead.

Each scenario is a Python file under ``tests/fixtures`` rather than a string
literal, so lint, type checking and docstring coverage see it as code. Run a
module and it executes the scenario; import it, as pytest's
``--doctest-modules`` does, and its ``__main__`` guard keeps the body inert.

The child is launched as ``python -m tests.fixtures.<scenario>`` from the
repository root, not by file path. A by-path launch puts the scenario's own
directory on the child's import path instead of the root, so the scenario's
``tests.fixtures.scenario_support`` import would depend on the environment
already exposing the checkout. ``-m`` puts the working directory on the path,
which is the root by construction.

A marker on standard output is the completion signal. Scripts print it only
after the re-entrant log call inside ``__del__`` returns, so a script that
merely avoids crashing without finishing the re-entry still fails. For the
same reason each finalizer records completion after its log call, never
before: Python swallows an exception raised by ``__del__``, and a flag set
first would let the marker stand for a re-entry that never happened.
"""

from __future__ import annotations

import os
import subprocess  # ruff: ignore[suspicious-subprocess-import] fixed argv, no shell
import sys
from pathlib import Path

#: How long a scenario may take before it is treated as deadlocked.
CHILD_TIMEOUT_SECONDS = 10.0

#: Directory holding the scenario scripts.
FIXTURES = Path(__file__).resolve().parent / "fixtures"

#: Repository root, the working directory and import root of each child.
REPOSITORY_ROOT = Path(__file__).resolve().parent.parent

CLEAR_HANDLERS_SCENARIO = FIXTURES / "reentrant_clear_handlers.py"
CLEAR_HANDLERS_MARKER = "clear-handlers-finalizer-complete"

REMOVE_HANDLER_SCENARIO = FIXTURES / "reentrant_remove_handler.py"
REMOVE_HANDLER_MARKER = "remove-handler-finalizer-complete"

BASIC_CONFIG_SCENARIO = FIXTURES / "reentrant_basic_config.py"
BASIC_CONFIG_MARKER = "basic-config-finalizer-complete"

CLEAR_FILTERS_SCENARIO = FIXTURES / "reentrant_clear_filters.py"
CLEAR_FILTERS_MARKER = "clear-filters-finalizer-complete"

REPLACE_CONFIGURATION_SCENARIO = FIXTURES / "reentrant_replace_configuration.py"
REPLACE_CONFIGURATION_MARKER = "replace-configuration-finalizer-complete"

RESET_MANAGER_SCENARIO = FIXTURES / "reentrant_reset_manager.py"
RESET_MANAGER_MARKER = "reset-manager-finalizer-complete"


def run_reentrant_finalizer_script(
    script: Path,
    marker: str,
    *,
    timeout: float = CHILD_TIMEOUT_SECONDS,
) -> None:
    """Run *script* in a child interpreter and require a *marker* output line.

    Parameters
    ----------
    script
        Path to the scenario script, run as a module from the repository root.
    marker
        Exact line the script prints after its re-entrant call returns.
    timeout
        Seconds the child may run before it is killed and the test fails.

    Raises
    ------
    AssertionError
        If the child times out, exits non-zero, or does not print ``marker``.
        The captured standard output and error are included so a deadlock is
        distinguishable from an exception in the scenario.

    Examples
    --------
    >>> run_reentrant_finalizer_script(  # doctest: +SKIP
    ...     CLEAR_HANDLERS_SCENARIO,
    ...     CLEAR_HANDLERS_MARKER,
    ... )

    The example is skipped under ``--doctest-modules`` so the dedicated test
    remains the single owner of the scenario's timeout budget.
    """
    try:
        result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed argv, no shell
            [sys.executable, "-m", _scenario_module(script)],
            capture_output=True,
            encoding="utf-8",
            errors="replace",
            env=_child_environment(),
            cwd=REPOSITORY_ROOT,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        message = (
            f"child interpreter did not finish within {timeout}s; "
            f"the finalizer most likely deadlocked\n"
            f"stdout:\n{_as_text(exc.stdout)}\nstderr:\n{_as_text(exc.stderr)}"
        )
        raise AssertionError(message) from exc

    # Explicit raises rather than `assert`: pytest can run under `-O`, which
    # removes assertion statements and would let a failed child run pass.
    if result.returncode != 0:
        message = (
            f"child interpreter exited with {result.returncode}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
        raise AssertionError(message)
    # The marker must be a whole line: a scenario that prints it embedded in
    # other text has not reached the point the marker stands for.
    if marker not in result.stdout.splitlines():
        message = (
            f"child interpreter never printed the completion marker {marker!r}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
        raise AssertionError(message)


def _scenario_module(script: Path) -> str:
    """Return the dotted module name the child runs *script* as.

    Parameters
    ----------
    script
        A scenario file inside :data:`REPOSITORY_ROOT`.

    Returns
    -------
    str
        The module path, such as ``tests.fixtures.reentrant_clear_handlers``.
    """
    relative = script.resolve().relative_to(REPOSITORY_ROOT).with_suffix("")
    return ".".join(relative.parts)


def _child_environment() -> dict[str, str]:
    """Return the child's environment with a fixed standard-stream encoding.

    The parent decodes the child's output as UTF-8 so it can compare the
    marker. An inherited ``PYTHONIOENCODING`` could make the child encode its
    streams differently, so the child is given an explicit value instead.

    Returns
    -------
    dict[str, str]
        The parent's environment with ``PYTHONIOENCODING`` fixed to UTF-8.
    """
    environment = os.environ.copy()
    environment["PYTHONIOENCODING"] = "utf-8"
    return environment


def _as_text(output: str | bytes | None) -> str:
    """Decode captured child output for an assertion message."""
    match output:
        case None:
            return "<none>"
        case bytes():
            return output.decode(errors="replace")
        case _:
            return output
