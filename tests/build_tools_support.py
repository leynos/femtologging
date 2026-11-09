"""Shared process setup for the build-tool contract tests.

Both the Make-wiring contracts and the check-script contracts need the same
repository root, a clean environment for fixed Make invocations, and throwaway
executables. Keeping those here stops the two suites from growing separate
copies that could drift apart.
"""

from __future__ import annotations

import os
import typing as typ
from pathlib import Path

ROOT: typ.Final[Path] = Path(__file__).resolve().parents[1]

# An outer Make's flags (`-i`, `-k`, `-n`, or a job server) would change how a
# nested fixed invocation fails, so they never reach these tests.
INHERITED_MAKE_VARIABLES: typ.Final[tuple[str, ...]] = (
    "MAKEFLAGS",
    "GNUMAKEFLAGS",
    "MFLAGS",
    "MAKELEVEL",
)


def make_environment(**extra: str) -> dict[str, str]:
    """Return an environment for a fixed Make invocation.

    Parameters
    ----------
    **extra
        Variables to add or replace.

    Returns
    -------
    dict[str, str]
        The parent environment without Make's inherited flag variables.

    Examples
    --------
    >>> make_environment(CARGO_LOG="cargo.log")["CARGO_LOG"]
    'cargo.log'
    """
    environment = {
        key: value
        for key, value in os.environ.items()
        if key not in INHERITED_MAKE_VARIABLES
    }
    environment.update(extra)
    return environment


def fake_executable(path: Path, body: str) -> Path:
    """Write an executable shell script for a probe.

    Parameters
    ----------
    path
        Destination file.
    body
        Shell body the script runs.

    Returns
    -------
    Path
        The executable path.

    Examples
    --------
    >>> import tempfile
    >>> with tempfile.TemporaryDirectory() as scratch:
    ...     fake_executable(Path(scratch) / "probe", "exit 0").name
    'probe'
    """
    path.write_text(f"#!/bin/sh\n{body}\n", "utf-8")
    path.chmod(0o755)
    return path
