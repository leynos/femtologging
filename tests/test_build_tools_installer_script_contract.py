"""Contract tests for ``scripts/install-build-tools.sh``.

The installer downloads a toolchain and installs a system package, so these
tests run it with a PATH holding only stub executables and the few system
tools it calls. Each case observes the exit status, the argument vectors the
stubs recorded, and the report on standard error; no host toolchain or package
manager is touched.
"""

from __future__ import annotations

import dataclasses
import re
import shlex
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the contract runs the repository script
import typing as typ

import pytest

from tests.build_tools_support import ROOT, fake_executable

if typ.TYPE_CHECKING:
    from pathlib import Path

SCRIPT: typ.Final[Path] = ROOT / "scripts" / "install-build-tools.sh"
TOOLCHAIN_CONFIG: typ.Final[Path] = ROOT / "rust-toolchain.toml"

# The installer reads the pin with sed and resolves its own directory with
# dirname; the stubs need nothing else beyond the shell builtins.
SYSTEM_TOOLS: typ.Final[tuple[str, ...]] = ("sed", "dirname")


def _channel() -> str:
    """Return the toolchain channel pinned by ``rust-toolchain.toml``."""
    config = TOOLCHAIN_CONFIG.read_text("utf-8")
    match = re.search(r'^channel = "(.*)"$', config, re.MULTILINE)
    assert match is not None, "rust-toolchain.toml must pin a channel"
    return match.group(1)


def _bin_dir(tmp_path: Path, *, needs_chmod: bool = False) -> Path:
    """Create the directory that becomes the installer's whole PATH.

    Parameters
    ----------
    tmp_path
        Per-test scratch directory.
    needs_chmod
        Add ``chmod`` for stubs that create an executable.

    Returns
    -------
    Path
        The prepared stub directory.
    """
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    tools = SYSTEM_TOOLS + (("chmod",) if needs_chmod else ())
    for tool in tools:
        resolved = shutil.which(tool)
        assert resolved is not None, f"{tool} must be available to run the probe"
        (bin_dir / tool).symlink_to(resolved)
    return bin_dir


def _environment(bin_dir: Path, *, host_os: str = "Linux") -> dict[str, str]:
    """Return an environment whose PATH resolves the stub tools only."""
    return {"PATH": str(bin_dir), "BUILD_TOOLS_OS": host_os}


def _run(environment: dict[str, str]) -> subprocess.CompletedProcess[str]:
    """Run the installer with ``environment`` and capture its report."""
    bash = shutil.which("bash")
    assert bash is not None, "bash must be available to run the installer"
    # ruff: ignore[subprocess-without-shell-equals-true] fixed argv, no shell
    return subprocess.run(
        [bash, str(SCRIPT)],
        env=environment,
        capture_output=True,
        text=True,
        check=False,
    )


def _recorded(log: Path) -> tuple[str, ...]:
    """Return the argv lines a stub recorded, or an empty tuple."""
    if not log.exists():
        return ()
    return tuple(log.read_text("utf-8").splitlines())


def _mold_creation_command(mold: Path) -> str:
    """Return a shell line that writes a working mold stub to ``mold``."""
    return (
        "printf '%s\\n' '#!/bin/sh' 'printf \"mold 2.41.0\\n\"' > "
        + shlex.quote(str(mold))
        + " && chmod +x "
        + shlex.quote(str(mold))
    )


def _stub_rustup(bin_dir: Path, log: Path, *, exit_status: int = 0) -> None:
    """Install a rustup stub that records its arguments and exits as asked."""
    body = "\n".join((
        f"printf '%s\\n' \"$@\" > {shlex.quote(str(log))}",
        f"exit {exit_status}",
    ))
    fake_executable(bin_dir / "rustup", body)


def _stub_id(bin_dir: Path, uid: int) -> None:
    """Install an id stub that reports ``uid``."""
    fake_executable(bin_dir / "id", f"printf '%s\\n' {uid}")


@dataclasses.dataclass(frozen=True, kw_only=True, slots=True)
class _PackageStub:
    """How one package-manager stub should behave and record its arguments."""

    name: str
    log: Path
    creates_mold: bool = False
    exit_status: int = 0


def _stub_package_manager(bin_dir: Path, stub: _PackageStub) -> None:
    """Install a package-manager stub that records its arguments."""
    lines = [f"printf '%s\\n' \"$@\" > {shlex.quote(str(stub.log))}"]
    if stub.creates_mold:
        lines.append(_mold_creation_command(bin_dir / "mold"))
    lines.append(f"exit {stub.exit_status}")
    fake_executable(bin_dir / stub.name, "\n".join(lines))


def _stub_tripwire(bin_dir: Path, name: str, log: Path) -> None:
    """Install a stub that records any invocation in ``log``."""
    fake_executable(bin_dir / name, f"printf '%s\\n' \"$@\" > {shlex.quote(str(log))}")


def _stub_sudo(bin_dir: Path, log: Path) -> None:
    """Install a sudo stub that accepts ``-n true`` and performs the install."""
    body = "\n".join((
        'if [ "$1" = -n ] && [ "$2" = true ]; then exit 0; fi',
        f"printf '%s\\n' \"$@\" > {shlex.quote(str(log))}",
        _mold_creation_command(bin_dir / "mold"),
        "exit 0",
    ))
    fake_executable(bin_dir / "sudo", body)


MANAGER_CASES: typ.Final[tuple[tuple[str, tuple[str, ...]], ...]] = (
    ("dnf", ("install", "-y", "mold")),
    ("apt-get", ("install", "-y", "mold")),
    ("pacman", ("-S", "--noconfirm", "mold")),
)

MANAGER_IDS: typ.Final[tuple[str, ...]] = tuple(manager for manager, _ in MANAGER_CASES)


@pytest.mark.parametrize(("manager", "arguments"), MANAGER_CASES, ids=MANAGER_IDS)
def test_linux_root_install_records_the_channel_and_package(
    manager: str, arguments: tuple[str, ...], tmp_path: Path
) -> None:
    """A root host gets the pinned toolchain and the selected mold package."""
    bin_dir = _bin_dir(tmp_path, needs_chmod=True)
    rustup_log = tmp_path / "rustup.log"
    package_log = tmp_path / "package.log"
    _stub_rustup(bin_dir, rustup_log)
    _stub_id(bin_dir, 0)
    _stub_package_manager(
        bin_dir, _PackageStub(name=manager, log=package_log, creates_mold=True)
    )

    result = _run(_environment(bin_dir))

    expected_rustup = (
        "toolchain",
        "install",
        _channel(),
        "--profile",
        "minimal",
        "--component",
        "rustfmt",
        "--component",
        "clippy",
        "--component",
        "rust-analyzer",
    )
    assert result.returncode == 0, result.stderr
    assert _recorded(rustup_log) == expected_rustup, (
        "the pinned toolchain and its components must be installed"
    )
    assert _recorded(package_log) == arguments, (
        f"{manager} must install the mold package"
    )
    assert (bin_dir / "mold").is_file(), "the package must leave a usable mold"
    assert "mold installed" in result.stderr, result.stderr


def test_existing_mold_is_reported_without_installing_a_package(
    tmp_path: Path,
) -> None:
    """A usable mold skips package installation entirely."""
    bin_dir = _bin_dir(tmp_path)
    package_log = tmp_path / "package.log"
    _stub_rustup(bin_dir, tmp_path / "rustup.log")
    fake_executable(bin_dir / "mold", "printf 'mold 2.41.0\\n'")
    _stub_tripwire(bin_dir, "dnf", package_log)

    result = _run(_environment(bin_dir))

    assert result.returncode == 0, result.stderr
    assert "mold already installed" in result.stderr, result.stderr
    assert not _recorded(package_log), "no package manager may run"


def test_non_linux_host_skips_mold(tmp_path: Path) -> None:
    """A non-Linux host installs no mold and reports the skip."""
    bin_dir = _bin_dir(tmp_path)
    package_log = tmp_path / "package.log"
    _stub_rustup(bin_dir, tmp_path / "rustup.log")
    _stub_tripwire(bin_dir, "dnf", package_log)

    result = _run(_environment(bin_dir, host_os="Darwin"))

    assert result.returncode == 0, result.stderr
    assert "Linux-only" in result.stderr, result.stderr
    assert "skipping on Darwin" in result.stderr, result.stderr
    assert not _recorded(package_log), "no package manager may run on Darwin"


def test_missing_rustup_fails_with_the_install_hint(tmp_path: Path) -> None:
    """No rustup on PATH stops the installer with the rustup.rs hint."""
    bin_dir = _bin_dir(tmp_path)

    result = _run(_environment(bin_dir))

    assert result.returncode != 0, "a missing rustup must fail the installer"
    assert "rustup not found" in result.stderr, result.stderr
    assert "rustup.rs" in result.stderr, result.stderr


def test_failed_toolchain_install_stops_the_installer(tmp_path: Path) -> None:
    """A rustup failure is reported as a failed toolchain installation."""
    bin_dir = _bin_dir(tmp_path)
    rustup_log = tmp_path / "rustup.log"
    _stub_rustup(bin_dir, rustup_log, exit_status=1)

    result = _run(_environment(bin_dir))

    assert result.returncode != 0, "a failed rustup must fail the installer"
    assert "failed to install toolchain" in result.stderr, result.stderr
    assert _channel() in _recorded(rustup_log), (
        "rustup must be invoked for the pinned channel"
    )


def test_unavailable_package_install_prints_the_command(tmp_path: Path) -> None:
    """Without root or passwordless sudo the administrator command is shown."""
    bin_dir = _bin_dir(tmp_path)
    package_log = tmp_path / "package.log"
    _stub_rustup(bin_dir, tmp_path / "rustup.log")
    _stub_id(bin_dir, 1000)
    _stub_tripwire(bin_dir, "dnf", package_log)

    result = _run(_environment(bin_dir))

    assert result.returncode != 0, "an unavailable package install must fail"
    assert "automatic installation is unavailable" in result.stderr, result.stderr
    assert "dnf install -y mold" in result.stderr, result.stderr
    assert not _recorded(package_log), "the manager must not run without privilege"


def test_package_install_without_a_working_mold_fails(tmp_path: Path) -> None:
    """A successful package command that leaves mold unusable is an error."""
    bin_dir = _bin_dir(tmp_path)
    _stub_rustup(bin_dir, tmp_path / "rustup.log")
    _stub_id(bin_dir, 0)
    _stub_package_manager(
        bin_dir, _PackageStub(name="dnf", log=tmp_path / "package.log")
    )

    result = _run(_environment(bin_dir))

    assert result.returncode != 0, "an unusable mold must fail the installer"
    assert "still unavailable after installation" in result.stderr, result.stderr


def test_passwordless_sudo_installs_mold(tmp_path: Path) -> None:
    """Passwordless sudo runs the package command and installs mold."""
    bin_dir = _bin_dir(tmp_path, needs_chmod=True)
    sudo_log = tmp_path / "sudo.log"
    package_log = tmp_path / "package.log"
    _stub_rustup(bin_dir, tmp_path / "rustup.log")
    _stub_id(bin_dir, 1000)
    _stub_tripwire(bin_dir, "dnf", package_log)
    _stub_sudo(bin_dir, sudo_log)

    result = _run(_environment(bin_dir))

    assert result.returncode == 0, result.stderr
    assert _recorded(sudo_log) == ("-n", "dnf", "install", "-y", "mold"), (
        "sudo must run the non-interactive package command"
    )
    assert not _recorded(package_log), "the manager must run only through sudo"
    assert "mold installed" in result.stderr, result.stderr
