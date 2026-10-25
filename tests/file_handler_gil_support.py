"""Parent-side runner for the file-handler interpreter-lock scenarios.

Interpreter-lock behaviour cannot be observed from inside a single interpreter,
and an in-process test that deadlocks hangs the whole suite instead of failing.
This module runs each scenario -- rendered by
:mod:`tests.file_handler_gil_scenarios` -- in a child interpreter under a
timeout, and reports a child that never finishes as a probable GIL deadlock
rather than blocking collection.

Detecting the defect needs care, because a wait that holds the GIL and one that
releases it are indistinguishable from the caller's own point of view. Each
scenario parks the file worker inside a formatter, starts a watchdog thread
that records a progress timestamp every few milliseconds, and then makes the
blocking call from a second thread.

The verdict counts progress records that landed *inside* the call's own window,
between the moment the calling thread entered the call and the moment it
returned:

- If the binding releases the GIL, the watchdog runs freely for the whole wait
  -- a second or more -- and records hundreds of timestamps in that window.
- If the binding holds the GIL, the watchdog is frozen for the whole wait. A
  blocked Rust binding returns to the interpreter only when its wait ends, so
  the watchdog cannot complete even one spin of its loop.

Counting avoids a race that a bare ordering check would have. The caller
records its window boundary just before the call, so a check that merely asked
"did the watchdog run before the call returned?" could be satisfied by a
watchdog that ran *before* the call started. Requiring several records inside
the window cannot: a GIL-holding wait yields none.
"""

from __future__ import annotations

import subprocess  # ruff: ignore[suspicious-subprocess-import] scenarios are isolated in child interpreters
import sys
import typing as typ

import pytest

from .file_handler_gil_scenarios import render_scenario

if typ.TYPE_CHECKING:
    from pathlib import Path

#: Wall-clock budget for one scenario, including interpreter start-up. This
#: must stay under the suite-wide ``timeout = 30`` in ``pyproject.toml``: if
#: pytest-timeout fired first it would kill the child and report a bare
#: timeout, hiding the probable-GIL-deadlock diagnosis below.
SCENARIO_TIMEOUT_SECONDS = 20.0


def _run_scenario(script: str, *, context: str) -> None:
    """Run ``script`` in a fresh interpreter, failing the test on any error.

    Passing the script with ``-c`` keeps the scenario self-contained and avoids
    creating a temporary script file.
    """
    try:
        # ruff: ignore[subprocess-without-shell-equals-true] fixed interpreter
        # and repository-rendered test scenario
        completed = subprocess.run(
            [sys.executable, "-c", script],
            capture_output=True,
            text=True,
            timeout=SCENARIO_TIMEOUT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired:
        pytest.fail(
            f"{context}: scenario did not finish within "
            f"{SCENARIO_TIMEOUT_SECONDS:g}s, so a blocking handler call is "
            "probably holding the GIL."
        )
    if completed.returncode != 0:
        pytest.fail(
            f"{context}: scenario failed in the child interpreter (exit "
            f"{completed.returncode}):\n{completed.stdout}\n{completed.stderr}"
        )


def run_scenario(builder_name: str, body_name: str, path: Path) -> None:
    """Run a named scenario against one builder in a child interpreter.

    Parameters
    ----------
    builder_name : str
        Registered builder label, such as ``"file"`` or ``"rotating"``.
    body_name : str
        Registered scenario label, such as ``"handle-block"`` or ``"flush"``.
    path : pathlib.Path
        Output path passed to the child scenario.

    The helper returns no value after the child exits successfully. A timeout
    or non-zero exit fails the calling test and includes the child's
    diagnostic output.

    Examples
    --------
    Run the bounded enqueue scenario in a fresh interpreter:

    >>> from pathlib import Path
    >>> run_scenario("file", "handle-timeout", Path("timeout.log"))
    """
    _run_scenario(
        render_scenario(builder_name, body_name, path),
        context=f"{builder_name} handler.{body_name} ({path.name})",
    )
