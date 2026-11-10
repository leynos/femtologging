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
    for tool in ("sed", "grep", "uname", "dirname"):
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


def _stub_rustup(bin_dir: Path, listed: str) -> None:
    """Install a rustup stub that lists `listed` as an installed toolchain."""
    fake_executable(
        bin_dir / "rustup",
        'if [ "$1" = toolchain ] && [ "$2" = list ]; then '
        f"printf '{listed}\\n'; exit 0; fi; exit 1",
    )


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class _Case:
    """One host capability scenario and the report it must produce."""

    toolchain_present: bool
    mold_body: str | None
    host_os: str
    returncode: int
    messages: tuple[str, ...]


CASES: typ.Final[tuple[_Case, ...]] = (
    _Case(
        toolchain_present=True,
        mold_body="printf 'mold 2.41.0\\n'",
        host_os="Linux",
        returncode=0,
        messages=("mold at",),
    ),
    _Case(
        toolchain_present=True,
        mold_body=None,
        host_os="Linux",
        returncode=1,
        messages=("mold not found", "make install-build-tools"),
    ),
    _Case(
        toolchain_present=False,
        mold_body="printf 'mold 2.41.0\\n'",
        host_os="Linux",
        returncode=1,
        messages=("is not installed",),
    ),
    _Case(
        toolchain_present=True,
        mold_body=None,
        host_os="Darwin",
        returncode=0,
        messages=("Linux-only",),
    ),
    _Case(
        toolchain_present=True,
        mold_body="exit 2",
        host_os="Linux",
        returncode=1,
        messages=("cannot run",),
    ),
)

CASE_IDS: typ.Final[tuple[str, ...]] = (
    "both-present",
    "mold-missing",
    "toolchain-missing",
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
    suffix = "x86_64-unknown-linux-gnu (active)"
    listed = f"{_channel()}-{suffix}" if case.toolchain_present else f"stable-{suffix}"
    _stub_rustup(bin_dir, listed)
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
