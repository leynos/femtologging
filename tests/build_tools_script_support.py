"""Rustup stub helpers for the build-tool preflight contract tests.

The preflight script runs two rustup probes, so its contract tests need a stub
that can vary each probe's answer and record what the script asked. Keeping
the stub here leaves the contract module room for its scenarios under the
repository's 400-line module cap.
"""

from __future__ import annotations

import dataclasses
import typing as typ

from tests.build_tools_support import fake_executable

if typ.TYPE_CHECKING:
    from pathlib import Path

HOST_TRIPLE: typ.Final[str] = "x86_64-unknown-linux-gnu"


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class StubOptions:
    """Per-probe answers and recording targets for the rustup stub.

    Parameters
    ----------
    pwd_log
        When given, the stub records its working directory here for each
        command it serves.
    argv_log
        When given, the stub appends each command's argument vector here.
    host_triple
        The triple `rustup show` reports, or None when the probe must fail.
    reports_host
        Whether `rustup show` prints a `Default host:` line; False omits it.
    listing_exit_status
        Exit status for `rustup toolchain list`; non-zero prints no listing.
    """

    pwd_log: Path | None = None
    argv_log: Path | None = None
    host_triple: str | None = HOST_TRIPLE
    reports_host: bool = True
    listing_exit_status: int = 0


def stub_rustup(
    bin_dir: Path,
    listed: str,
    options: StubOptions | None = None,
) -> None:
    """Install a rustup stub that serves the two probes the script runs.

    Parameters
    ----------
    bin_dir
        Directory holding the stub executables.
    listed
        The single line the stub reports for `rustup toolchain list`.
    options
        Per-probe answers and recording targets; defaults to the test host.
    """
    options = options or StubOptions()
    record = ""
    if options.argv_log is not None:
        record += f"printf '%s\\n' \"$*\" >> '{options.argv_log}'; "
    if options.pwd_log is not None:
        record += f"pwd >> '{options.pwd_log}'; "
    if options.host_triple is None:
        shown = "exit 1"
    elif options.reports_host:
        shown = f"printf 'Default host: {options.host_triple}\\n'; exit 0"
    else:
        shown = "printf 'Default toolchain: stable\\n'; exit 0"
    listing = f"printf '{listed}\\n'; exit 0"
    if options.listing_exit_status:
        listing = f"exit {options.listing_exit_status}"
    fake_executable(
        bin_dir / "rustup",
        'if [ "$1" = toolchain ] && [ "$2" = list ]; then '
        f"{record}{listing}; fi; "
        f'if [ "$1" = show ]; then {record}{shown}; fi; '
        "exit 1",
    )
