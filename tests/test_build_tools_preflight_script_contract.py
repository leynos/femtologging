"""Contract tests for the build-tool check script.

The Make preflight delegates to ``scripts/check-build-tools.sh``. These tests
run that script against stub tools, so each missing or unusable prerequisite
is exercised without depending on the host's installed toolchain or linker.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the contract runs the repository script
import typing as typ

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


def _preflight_environment(bin_dir: Path, *, host_os: str = "Linux") -> dict[str, str]:
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


def test_the_preflight_accepts_the_pinned_toolchain_and_mold(
    tmp_path: Path,
) -> None:
    """A host with both prerequisites passes the capability check."""
    bin_dir = _probe_bin_dir(tmp_path)
    _stub_rustup(bin_dir, f"{_channel()}-x86_64-unknown-linux-gnu (active)")
    fake_executable(bin_dir / "mold", "printf 'mold 2.41.0\\n'")
    result = _run_preflight(_preflight_environment(bin_dir))
    assert result.returncode == 0, result.stderr
    assert "mold at" in result.stderr, result.stderr


def test_the_preflight_names_the_install_command_when_mold_is_missing(
    tmp_path: Path,
) -> None:
    """A missing Linux linker fails with the actionable installation command."""
    bin_dir = _probe_bin_dir(tmp_path)
    _stub_rustup(bin_dir, f"{_channel()}-x86_64-unknown-linux-gnu (active)")
    result = _run_preflight(_preflight_environment(bin_dir))
    assert result.returncode == 1, result.stderr
    assert "mold not found" in result.stderr, result.stderr
    assert "make install-build-tools" in result.stderr, result.stderr


def test_the_preflight_fails_when_the_pinned_toolchain_is_absent(
    tmp_path: Path,
) -> None:
    """An uninstalled pin fails instead of letting Cargo pick another channel."""
    bin_dir = _probe_bin_dir(tmp_path)
    _stub_rustup(bin_dir, "stable-x86_64-unknown-linux-gnu (active)")
    fake_executable(bin_dir / "mold", "printf 'mold 2.41.0\\n'")
    result = _run_preflight(_preflight_environment(bin_dir))
    assert result.returncode == 1, result.stderr
    assert "is not installed" in result.stderr, result.stderr


def test_the_preflight_tolerates_a_platform_without_mold(tmp_path: Path) -> None:
    """A non-Linux host uses its platform linker and passes the check."""
    bin_dir = _probe_bin_dir(tmp_path)
    _stub_rustup(bin_dir, f"{_channel()}-x86_64-unknown-linux-gnu (active)")
    result = _run_preflight(_preflight_environment(bin_dir, host_os="Darwin"))
    assert result.returncode == 0, result.stderr
    assert "Linux-only" in result.stderr, result.stderr


def test_the_preflight_propagates_a_broken_linker(tmp_path: Path) -> None:
    """A mold that cannot report its version is treated as unavailable."""
    bin_dir = _probe_bin_dir(tmp_path)
    _stub_rustup(bin_dir, f"{_channel()}-x86_64-unknown-linux-gnu (active)")
    fake_executable(bin_dir / "mold", "exit 2")
    result = _run_preflight(_preflight_environment(bin_dir))
    assert result.returncode == 1, result.stderr
    assert "cannot run" in result.stderr, result.stderr
