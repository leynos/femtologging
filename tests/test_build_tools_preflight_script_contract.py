"""Contract tests for the build-tool check script.

The Make preflight delegates to ``scripts/check-build-tools.sh``. These tests
run that script against stub tools, so each missing or unusable prerequisite
is exercised without depending on the host's installed toolchain or linker.
"""

from __future__ import annotations

import dataclasses
import os
import re
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the contract runs the repository script
import typing as typ

import pytest

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
    """
    for tool in ("sed", "uname", "dirname"):
        resolved = shutil.which(tool)
        assert resolved is not None, f"{tool} must be available to run the probe"
        (bin_dir / tool).symlink_to(resolved)


def _probe_bin_dir(tmp_path: Path) -> Path:
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


def _preflight_environment(bin_dir: Path, *, host_os: str) -> dict[str, str]:
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


def _run_preflight(environment: dict[str, str]) -> subprocess.CompletedProcess[str]:
    """Run the check script with `environment` and capture its report.

    Parameters
    ----------
    environment
        Environment built by `_preflight_environment`.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The completed probe.
    """
    bash = shutil.which("bash")
    assert bash is not None, "bash must be available to run the probe"
    # ruff: ignore[subprocess-without-shell-equals-true] the argument vector is
    # fixed here and the script path is the repository's own
    return subprocess.run(
        [bash, str(ROOT / "scripts" / "check-build-tools.sh")],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )


def _channel() -> str:
    """Return the toolchain pin the preflight reads from the repository."""
    config = (ROOT / "rust-toolchain.toml").read_text("utf-8")
    match = re.search(r'^channel = "(.*)"$', config, re.MULTILINE)
    assert match is not None, "rust-toolchain.toml must pin a channel"
    return match.group(1)


def _installed(channel: str) -> str:
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


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class _Case:
    """One host capability scenario and the report it must produce."""

    listed_toolchain: str
    mold_body: str | None
    host_os: str
    returncode: int
    messages: tuple[str, ...]
    absent_messages: tuple[str, ...] = ()
    host_triple: str | None = HOST_TRIPLE
    reports_host: bool = True
    listing_exit_status: int = 0


def _cases() -> tuple[_Case, ...]:
    """Return the capability scenarios, pinned to the repository's channel."""
    channel = _channel()
    return (
        _Case(
            listed_toolchain=_installed(channel),
            mold_body="printf 'mold 2.41.0\\n'",
            host_os="Linux",
            returncode=0,
            messages=("mold at",),
        ),
        _Case(
            listed_toolchain=_installed(channel),
            mold_body=None,
            host_os="Linux",
            returncode=1,
            messages=("mold not found", "make install-build-tools"),
        ),
        _Case(
            listed_toolchain=_installed("stable"),
            mold_body="printf 'mold 2.41.0\\n'",
            host_os="Linux",
            returncode=1,
            messages=("is not installed",),
        ),
        _Case(
            listed_toolchain=_installed(f"{channel}0"),
            mold_body="printf 'mold 2.41.0\\n'",
            host_os="Linux",
            returncode=1,
            messages=("is not installed",),
        ),
        _Case(
            listed_toolchain=f"{channel}-aarch64-unknown-linux-gnu (installed)",
            mold_body="printf 'mold 2.41.0\\n'",
            host_os="Linux",
            returncode=1,
            messages=("is not installed",),
        ),
        _Case(
            listed_toolchain="",
            mold_body="printf 'mold 2.41.0\\n'",
            host_os="Linux",
            returncode=1,
            messages=("rustup could not list installed toolchains",),
            absent_messages=("is not installed",),
            listing_exit_status=1,
        ),
        _Case(
            listed_toolchain=f"{channel} (installed)",
            mold_body="printf 'mold 2.41.0\\n'",
            host_os="Linux",
            returncode=0,
            messages=("rustup show failed", "mold at"),
            host_triple=None,
        ),
        _Case(
            listed_toolchain=f"{channel} (installed)",
            mold_body="printf 'mold 2.41.0\\n'",
            host_os="Linux",
            returncode=0,
            messages=("rustup show reported no default host triple", "mold at"),
            reports_host=False,
        ),
        _Case(
            listed_toolchain=_installed(channel),
            mold_body=None,
            host_os="Darwin",
            returncode=0,
            messages=("Linux-only",),
        ),
        _Case(
            listed_toolchain=_installed(channel),
            mold_body="exit 2",
            host_os="Linux",
            returncode=1,
            messages=("cannot run",),
        ),
    )


CASES: typ.Final[tuple[_Case, ...]] = _cases()

CASE_IDS: typ.Final[tuple[str, ...]] = (
    "both-present",
    "mold-missing",
    "toolchain-missing",
    "toolchain-prefix-collision",
    "toolchain-other-host",
    "toolchain-listing-failed",
    "toolchain-host-unresolved",
    "toolchain-host-unreported",
    "non-linux",
    "broken-mold",
)


def _run_case(tmp_path: Path, case: _Case) -> subprocess.CompletedProcess[str]:
    """Run the preflight against the stubs `case` describes.

    Parameters
    ----------
    tmp_path
        Per-test scratch directory.
    case
        The capability scenario.

    Returns
    -------
    subprocess.CompletedProcess[str]
        The probe's completed run.
    """
    bin_dir = _probe_bin_dir(tmp_path)
    stub_rustup(
        bin_dir,
        case.listed_toolchain,
        StubOptions(
            host_triple=case.host_triple,
            reports_host=case.reports_host,
            listing_exit_status=case.listing_exit_status,
        ),
    )
    if case.mold_body is not None:
        fake_executable(bin_dir / "mold", case.mold_body)
    return _run_preflight(_preflight_environment(bin_dir, host_os=case.host_os))


@pytest.mark.parametrize("case", CASES, ids=CASE_IDS)
def test_the_preflight_judges_each_host_capability(case: _Case, tmp_path: Path) -> None:
    """A present, missing or unusable prerequisite produces its own report."""
    result = _run_case(tmp_path, case)
    assert result.returncode == case.returncode, result.stderr
    for message in case.messages:
        assert message in result.stderr, result.stderr
    for message in case.absent_messages:
        assert message not in result.stderr, result.stderr


def test_the_toolchain_probes_run_outside_the_repository(tmp_path: Path) -> None:
    """Neither rustup probe can resolve the repository's toolchain override."""
    bin_dir = _probe_bin_dir(tmp_path)
    pwd_log = tmp_path / "rustup-pwd"
    stub_rustup(bin_dir, _installed(_channel()), StubOptions(pwd_log=pwd_log))
    fake_executable(bin_dir / "mold", "printf 'mold 2.41.0\\n'")

    result = _run_preflight(_preflight_environment(bin_dir, host_os="Linux"))

    assert result.returncode == 0, result.stderr
    records = pwd_log.read_text("utf-8").splitlines()
    assert len(records) == 2, (
        f"both rustup probes must report their directory: {records}"
    )
    assert set(records) == {"/"}, "rustup must run outside the repository"


def test_the_rustup_probes_run_the_expected_subcommands(tmp_path: Path) -> None:
    """The preflight asks for the host first, then the installed listing."""
    bin_dir = _probe_bin_dir(tmp_path)
    argv_log = tmp_path / "rustup-argv"
    stub_rustup(bin_dir, _installed(_channel()), StubOptions(argv_log=argv_log))
    fake_executable(bin_dir / "mold", "printf 'mold 2.41.0\\n'")

    result = _run_preflight(_preflight_environment(bin_dir, host_os="Linux"))

    assert result.returncode == 0, result.stderr
    commands = [line.split() for line in argv_log.read_text("utf-8").splitlines()]
    assert commands == [["show"], ["toolchain", "list"]], commands


def test_a_failed_listing_is_reported_as_a_probe_failure(tmp_path: Path) -> None:
    """A failed listing exits 1 and is not described as an absent toolchain."""
    bin_dir = _probe_bin_dir(tmp_path)
    stub_rustup(bin_dir, "", StubOptions(listing_exit_status=1))
    fake_executable(bin_dir / "mold", "printf 'mold 2.41.0\\n'")

    result = _run_preflight(_preflight_environment(bin_dir, host_os="Linux"))

    assert result.returncode == 1, result.stderr
    assert "rustup could not list installed toolchains" in result.stderr
    assert "is not installed" not in result.stderr


def test_a_failed_show_is_named_with_the_bare_channel(tmp_path: Path) -> None:
    """A failed `rustup show` is named and the bare channel still satisfies it."""
    bin_dir = _probe_bin_dir(tmp_path)
    channel = _channel()
    stub_rustup(bin_dir, f"{channel} (installed)", StubOptions(host_triple=None))
    fake_executable(bin_dir / "mold", "printf 'mold 2.41.0\\n'")

    result = _run_preflight(_preflight_environment(bin_dir, host_os="Linux"))

    assert result.returncode == 0, result.stderr
    assert "rustup show failed" in result.stderr
    assert f"toolchain {channel} available" in result.stderr
