"""Run a workflow step's shell script against stubbed external commands.

A workflow step's ``run`` body is a shell script, and the question the
feature-matrix contract asks of it is behavioural: if the driver fails, does
the step fail? Answering that by reading the text means modelling bash —
quoting, command separators, redirections, here-documents, ``set`` option
order, conditional lists, and the shell template's own flags. Every model of
the shell is a list of the constructs it does not handle, and each gap is a
script that passes the contract while discarding the driver's verdict.

So bash answers the question instead. The script is written to a file and
executed under the shell template the workflow declares, with ``cargo``,
``make`` and ``uv`` replaced by stubs on ``PATH``. Nothing is compiled, and
what the step does with a failing command is observed rather than inferred.
"""

from __future__ import annotations

import dataclasses as dc
import os
import shlex
import subprocess  # ruff: ignore[suspicious-subprocess-import] the contract runs a fixed bash
import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

#: Commands the step's script invokes but must not really run here.
STUBBED_COMMANDS: typ.Final[tuple[str, ...]] = ("cargo", "make", "uv")

#: The shell GitHub uses for a ``run`` body on Linux when none is declared.
DEFAULT_SHELL: typ.Final[tuple[str, ...]] = (
    "bash",
    "--noprofile",
    "--norc",
    "-eo",
    "pipefail",
)

#: A stub that records its invocation and reports a controllable status.
_STUB: typ.Final[str] = """#!/bin/sh
printf '%s\\n' "$*" >> "$STEP_LOG"
case "$*" in
  *"{runner}"*) exit "$DRIVER_EXIT" ;;
esac
exit 0
"""


def shell_words(shell: object) -> list[str]:
    """Return the command words that run a script under ``shell``.

    ``None`` and the built-in ``bash`` and ``sh`` names mean GitHub's
    fail-fast default for the platform. A custom template is split and its
    ``{0}`` placeholder is replaced by the script path when the script is
    run, so the template's own flags — including a disabling or no-execution
    one — take effect exactly as they would in the workflow.

    Returns
    -------
        The shell command, ending in ``{0}``.

    Raises
    ------
    ValueError
        If a custom template has no ``{0}`` placeholder.
    """
    if shell is None or shell in {"bash", "sh"}:
        return [*DEFAULT_SHELL, "{0}"]
    words = shlex.split(str(shell))
    if "{0}" not in words:
        message = f"the shell template {shell!r} has no {{0}} placeholder"
        raise ValueError(message)
    return words


def _write_stubs(bin_dir: Path, runner: str) -> None:
    """Write one stub per stubbed command into ``bin_dir``.

    Each stub appends its argument line to ``$STEP_LOG``, which
    :func:`run_script` sets for the child, and exits with ``$DRIVER_EXIT``
    when the line names the driver.

    Parameters
    ----------
    bin_dir
        The directory to place the stubs in, first on the child's ``PATH``.
    runner
        The driver path whose invocation reports ``DRIVER_EXIT``.
    """
    body = _STUB.replace("{runner}", runner)
    for name in STUBBED_COMMANDS:
        stub = bin_dir / name
        stub.write_text(body, encoding="utf-8")
        stub.chmod(0o755)


@dc.dataclass(frozen=True, slots=True)
class StepRun:
    """Everything one run of a step's script needs."""

    script: str
    shell: object
    driver_exit: int
    runner: str
    workdir: Path


def run_script(run: StepRun) -> tuple[int, list[str]]:
    """Run a step's script and report its status and command log.

    Parameters
    ----------
    run
        The script, the shell template it runs under, the status the stubbed
        driver reports, the driver path the stub recognises, and a scratch
        directory for the script, the stubs and the log.

    Returns
    -------
    tuple[int, list[str]]
        The script's exit status and one argument line per stubbed call, in
        order.
    """
    bin_dir = run.workdir / "bin"
    bin_dir.mkdir()
    log = run.workdir / "calls.log"
    _write_stubs(bin_dir, run.runner)
    path = run.workdir / "step.sh"
    path.write_text(run.script, encoding="utf-8")
    command = [word.replace("{0}", str(path)) for word in shell_words(run.shell)]
    environment = {
        "PATH": f"{bin_dir}{os.pathsep}{os.environ.get('PATH', '')}",
        "STEP_LOG": str(log),
        "DRIVER_EXIT": str(run.driver_exit),
        "HOME": str(run.workdir),
    }
    result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed argv, no shell
        command,
        cwd=run.workdir,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )
    calls = log.read_text(encoding="utf-8").splitlines() if log.exists() else []
    return result.returncode, calls


def driver_calls(calls: cabc.Sequence[str], runner: str) -> list[str]:
    """Return the logged invocations that ran the driver.

    Returns
    -------
        One argument line per driver invocation, in order.
    """
    return [call for call in calls if runner in call]
