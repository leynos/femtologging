"""Contract tests for the build-tool check script.

The Make preflight delegates to ``scripts/check-build-tools.sh``. These tests
run that script against stub tools, so each missing or unusable prerequisite
is exercised without depending on the host's installed toolchain or linker.
"""

from __future__ import annotations

import dataclasses
import typing as typ

import pytest

from tests.build_tools_probe_support import (
    installed_line,
    pinned_channel,
    preflight_environment,
    probe_bin_dir,
    run_preflight,
)
from tests.build_tools_script_support import HOST_TRIPLE, StubOptions, stub_rustup
from tests.build_tools_support import fake_executable

if typ.TYPE_CHECKING:
    import subprocess
    from pathlib import Path


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class _Case:
    """One host capability scenario and the report it must produce."""

    name: str
    listed_toolchain: str
    mold_body: str | None
    host_os: str
    returncode: int
    messages: tuple[str, ...]
    absent_messages: tuple[str, ...] = ()
    host_triple: str | None = HOST_TRIPLE
    reports_host: bool = True
    listing_exit_status: int = 0


def _mold_cases(channel: str) -> tuple[_Case, ...]:
    """Return the scenarios that exercise the linker half of the check."""
    mol = "printf 'mold 2.41.0\\n'"
    return (
        _Case(
            name="both-present",
            listed_toolchain=installed_line(channel),
            mold_body=mol,
            host_os="Linux",
            returncode=0,
            messages=("mold at",),
        ),
        _Case(
            name="mold-missing",
            listed_toolchain=installed_line(channel),
            mold_body=None,
            host_os="Linux",
            returncode=1,
            messages=("mold not found", "make install-build-tools"),
        ),
        _Case(
            name="non-linux",
            listed_toolchain=installed_line(channel),
            mold_body=None,
            host_os="Darwin",
            returncode=0,
            messages=("Linux-only",),
        ),
        _Case(
            name="broken-mold",
            listed_toolchain=installed_line(channel),
            mold_body="exit 2",
            host_os="Linux",
            returncode=1,
            messages=("cannot run",),
        ),
    )


def _absent_toolchain_cases(channel: str) -> tuple[_Case, ...]:
    """Return the scenarios where the pinned channel is not installed."""
    mol = "printf 'mold 2.41.0\\n'"
    return (
        _Case(
            name="toolchain-missing",
            listed_toolchain=installed_line("stable"),
            mold_body=mol,
            host_os="Linux",
            returncode=1,
            messages=("is not installed",),
        ),
        _Case(
            name="toolchain-prefix-collision",
            listed_toolchain=installed_line(f"{channel}0"),
            mold_body=mol,
            host_os="Linux",
            returncode=1,
            messages=("is not installed",),
        ),
        _Case(
            name="toolchain-other-host",
            listed_toolchain=f"{channel}-aarch64-unknown-linux-gnu (installed)",
            mold_body=mol,
            host_os="Linux",
            returncode=1,
            messages=("is not installed",),
        ),
    )


def _probe_failure_cases(channel: str) -> tuple[_Case, ...]:
    """Return the scenarios where a rustup probe cannot answer."""
    mol = "printf 'mold 2.41.0\\n'"
    return (
        _Case(
            name="toolchain-listing-failed",
            listed_toolchain="",
            mold_body=mol,
            host_os="Linux",
            returncode=1,
            messages=("rustup could not list installed toolchains",),
            absent_messages=("is not installed",),
            listing_exit_status=1,
        ),
        _Case(
            name="toolchain-host-unresolved",
            listed_toolchain=f"{channel} (installed)",
            mold_body=mol,
            host_os="Linux",
            returncode=0,
            messages=("rustup show failed", "mold at"),
            host_triple=None,
        ),
        _Case(
            name="toolchain-host-unreported",
            listed_toolchain=f"{channel} (installed)",
            mold_body=mol,
            host_os="Linux",
            returncode=0,
            messages=("rustup show reported no default host triple", "mold at"),
            reports_host=False,
        ),
    )


def _cases() -> tuple[_Case, ...]:
    """Return the capability scenarios, pinned to the repository's channel."""
    channel = pinned_channel()
    return (
        *_mold_cases(channel),
        *_absent_toolchain_cases(channel),
        *_probe_failure_cases(channel),
    )


CASES: typ.Final[tuple[_Case, ...]] = _cases()

CASE_IDS: typ.Final[tuple[str, ...]] = tuple(case.name for case in CASES)


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
    bin_dir = probe_bin_dir(tmp_path)
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
    return run_preflight(preflight_environment(bin_dir, host_os=case.host_os))


@pytest.mark.parametrize("case", CASES, ids=CASE_IDS)
def test_the_preflight_judges_each_host_capability(case: _Case, tmp_path: Path) -> None:
    """A present, missing or unusable prerequisite produces its own report."""
    result = _run_case(tmp_path, case)
    assert result.returncode == case.returncode, result.stderr
    for message in case.messages:
        assert message in result.stderr, result.stderr
    for message in case.absent_messages:
        assert message not in result.stderr, result.stderr
