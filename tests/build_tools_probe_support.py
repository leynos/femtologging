"""Runners that execute the build-tool preflight against stub tools.

The capability scenarios and the probe-mechanics tests both have to start the
repository's check script with a PATH that resolves stubs rather than the
developer's tools. Keeping that machinery here lets each test module stay
inside the repository's module-length cap, and keeps the stub answers in
`build_tools_script_support` separate from the run itself.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the contract runs the repository script
import typing as typ

from tests.build_tools_script_support import HOST_TRIPLE, StubOptions, stub_rustup
from tests.build_tools_support import ROOT, fake_executable

if typ.TYPE_CHECKING:
    from pathlib import Path


def _link_system_tools(bin_dir: Path) -> None:
    """Link the few system tools the script runs into the stub directory.

    The real ``mold`` also lives under ``/usr/bin`` on some hosts, so the test
    PATH cannot include that directory: it would find the developer's linker
    and the missing-tool case would never be exercised.

    Parameters
    ----------
    bin_dir
        Directory that becomes the whole test PATH.

    Raises
    ------
    AssertionError
        If a system tool the script runs is not on the test host.
    """
    for tool in ("sed", "uname", "dirname"):
        resolved = shutil.which(tool)
        if resolved is None:
            msg = f"{tool} must be available to run the probe"
            raise AssertionError(msg)
        (bin_dir / tool).symlink_to(resolved)


def probe_bin_dir(tmp_path: Path) -> Path:
    """Create the directory that becomes the probe's whole PATH.

    Parameters
    ----------
    tmp_path
        Per-test scratch directory.

    Returns
    -------
    Path
        The prepared directory.
    """
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _link_system_tools(bin_dir)
    return bin_dir


def preflight_environment(bin_dir: Path, *, host_os: str) -> dict[str, str]:
    """Return an environment whose PATH resolves the stub tools only.

    Parameters
    ----------
    bin_dir
        Directory holding the stub executables and the linked system tools.
    host_os
        Value exported as ``BUILD_TOOLS_OS``.

    Returns
    -------
    dict[str, str]
        The environment for one preflight run.
    """
    environment = {
        key: value
        for key, value in os.environ.items()
        if key not in {"BUILD_TOOLS_OS", "PATH"}
    }
    environment["PATH"] = str(bin_dir)
    environment["BUILD_TOOLS_OS"] = host_os
    return environment


def run_preflight(environment: dict[str, str]) -> subprocess.CompletedProcess[str]:
    """Run the check script with `environment` and capture its report.

    Parameters
    ----------
    environment
        Environment built by `preflight_environment`.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The completed probe.

    Raises
    ------
    AssertionError
        If bash is not on the test host.
    """
    bash = shutil.which("bash")
    if bash is None:
        msg = "bash must be available to run the probe"
        raise AssertionError(msg)
    # ruff: ignore[subprocess-without-shell-equals-true] the argument vector is
    # fixed here and the script path is the repository's own
    return subprocess.run(
        [bash, str(ROOT / "scripts" / "check-build-tools.sh")],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )


def pinned_channel() -> str:
    """Return the toolchain pin the preflight reads from the repository.

    Returns
    -------
    str
        The channel `rust-toolchain.toml` pins.

    Raises
    ------
    AssertionError
        If the file records no channel line.
    """
    config = (ROOT / "rust-toolchain.toml").read_text("utf-8")
    match = re.search(r'^channel = "(.*)"$', config, re.MULTILINE)
    if match is None:
        msg = "rust-toolchain.toml must pin a channel"
        raise AssertionError(msg)
    return match.group(1)


def installed_line(channel: str) -> str:
    """Return the rustup listing line for `channel` on the test host.

    Parameters
    ----------
    channel
        Toolchain name to suffix with the host triple.

    Returns
    -------
    str
        One line in the form `rustup toolchain list` emits.
    """
    return f"{channel}-{HOST_TRIPLE} (active)"


def run_preflight_against(
    tmp_path: Path, listed: str, options: StubOptions | None = None
) -> subprocess.CompletedProcess[str]:
    """Run the preflight with `listed` as rustup's answer and a working mold.

    Parameters
    ----------
    tmp_path
        Per-test scratch directory.
    listed
        The line `rustup toolchain list` reports.
    options
        Per-probe answers and recording targets for the rustup stub.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The probe's completed run on the Linux route.
    """
    bin_dir = probe_bin_dir(tmp_path)
    stub_rustup(bin_dir, listed, options)
    fake_executable(bin_dir / "mold", "printf 'mold 2.41.0\\n'")
    return run_preflight(preflight_environment(bin_dir, host_os="Linux"))
