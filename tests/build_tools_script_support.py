"""Rustup stub helpers for the build-tool preflight contract tests.

The preflight script runs two rustup probes, so its contract tests need a stub
that can vary each probe's answer and record what the script asked. Keeping
the stub here leaves the contract module room for its scenarios under the
repository's 400-line module cap.
"""

from __future__ import annotations

import dataclasses
import shlex
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
    env_log
        When given, the stub appends the `RUSTUP_AUTO_INSTALL` value it was
        given here, so a test can prove the probe cannot provision.
    host_triple
        The triple `rustup show` reports, or None when the probe must fail.
    reports_host
        Whether `rustup show` prints a `Default host:` line; False omits it.
    show_exit_status
        Exit status for a `rustup show` that does report a host. A non-zero
        value reproduces a rustup that printed the host and then failed on the
        default toolchain it could not activate.
    listing_exit_status
        Exit status for `rustup toolchain list`; non-zero prints no listing.
    """

    pwd_log: Path | None = None
    argv_log: Path | None = None
    env_log: Path | None = None
    host_triple: str | None = HOST_TRIPLE
    reports_host: bool = True
    show_exit_status: int = 0
    listing_exit_status: int = 0


def _record_prefix(options: StubOptions) -> str:
    """Return the shell that records what the stub was given, if anything.

    Parameters
    ----------
    options
        The recording targets the caller asked for.

    Returns
    -------
    str
        One shell fragment per requested target, in a fixed order.

    Notes
    -----
    Each path is quoted as a shell argument rather than wrapped in literal
    single quotes: a test directory such as ``/tmp/O'Brien`` would otherwise
    close the quote and hand the stub a different command.
    """
    prefix = ""
    if options.argv_log is not None:
        destination = shlex.quote(str(options.argv_log))
        prefix += f"printf '%s\\n' \"$*\" >> {destination}; "
    if options.pwd_log is not None:
        prefix += f"pwd >> {shlex.quote(str(options.pwd_log))}; "
    if options.env_log is not None:
        destination = shlex.quote(str(options.env_log))
        prefix += (
            f"printf '%s\\n' \"${{RUSTUP_AUTO_INSTALL-unset}}\" >> {destination}; "
        )
    return prefix


def _show_body(options: StubOptions) -> str:
    """Return the shell that answers `rustup show`.

    Parameters
    ----------
    options
        The host answer to reproduce.

    Returns
    -------
    str
        A body that prints the requested line and exits with the requested
        status, or fails at once when no host is to be reported.
    """
    if options.host_triple is None:
        return "exit 1"
    if options.reports_host:
        line = f"printf 'Default host: {options.host_triple}\\n'"
    else:
        line = "printf 'Default toolchain: stable\\n'"
    return f"{line}; exit {options.show_exit_status}"


def _listing_body(listed: str, exit_status: int) -> str:
    """Return the shell that answers `rustup toolchain list`.

    Parameters
    ----------
    listed
        The single line to report.
    exit_status
        Status to exit with; non-zero reports no listing at all.

    Returns
    -------
    str
        The body for the listing probe.
    """
    if exit_status:
        return f"exit {exit_status}"
    return f"printf '{listed}\\n'; exit 0"


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
    record = _record_prefix(options)
    listing = _listing_body(listed, options.listing_exit_status)
    shown = _show_body(options)
    fake_executable(
        bin_dir / "rustup",
        'if [ "$1" = toolchain ] && [ "$2" = list ]; then '
        f"{record}{listing}; fi; "
        f'if [ "$1" = show ]; then {record}{shown}; fi; '
        "exit 1",
    )
