"""Contract tests for the preflight's rustup probes themselves.

The capability scenarios live beside the judgement they exercise. These tests
pin what the probes must do independently of any one host's answer: run
outside the repository, ask for the host before the listing, stay read-only,
and report a failed probe without mistaking it for an absent toolchain.
"""

from __future__ import annotations

import typing as typ

from tests.build_tools_probe_support import (
    installed_line,
    pinned_channel,
    run_preflight_against,
)
from tests.build_tools_script_support import StubOptions

if typ.TYPE_CHECKING:
    from pathlib import Path


def test_the_toolchain_probes_run_outside_the_repository(tmp_path: Path) -> None:
    """Neither rustup probe can resolve the repository's toolchain override."""
    pwd_log = tmp_path / "rustup-pwd"
    result = run_preflight_against(
        tmp_path, installed_line(pinned_channel()), StubOptions(pwd_log=pwd_log)
    )

    assert result.returncode == 0, result.stderr
    records = pwd_log.read_text("utf-8").splitlines()
    assert len(records) == 2, (
        f"both rustup probes must report their directory: {records}"
    )
    assert set(records) == {"/"}, "rustup must run outside the repository"


def test_the_rustup_probes_run_the_expected_subcommands(tmp_path: Path) -> None:
    """The preflight asks for the host first, then the installed listing."""
    argv_log = tmp_path / "rustup-argv"
    result = run_preflight_against(
        tmp_path, installed_line(pinned_channel()), StubOptions(argv_log=argv_log)
    )

    assert result.returncode == 0, result.stderr
    commands = [line.split() for line in argv_log.read_text("utf-8").splitlines()]
    assert commands == [["show"], ["toolchain", "list"]], commands


def test_a_failed_listing_is_reported_as_a_probe_failure(tmp_path: Path) -> None:
    """A failed listing exits 1 and is not described as an absent toolchain."""
    result = run_preflight_against(tmp_path, "", StubOptions(listing_exit_status=1))

    assert result.returncode == 1, result.stderr
    assert "rustup could not list installed toolchains" in result.stderr, result.stderr
    assert "is not installed" not in result.stderr, result.stderr


def test_a_failed_show_is_named_with_the_bare_channel(tmp_path: Path) -> None:
    """A failed `rustup show` is named and the bare channel still satisfies it."""
    channel = pinned_channel()
    result = run_preflight_against(
        tmp_path, f"{channel} (installed)", StubOptions(host_triple=None)
    )

    assert result.returncode == 0, result.stderr
    assert "rustup show failed" in result.stderr, result.stderr
    assert f"toolchain {channel} available" in result.stderr, result.stderr


def test_a_failed_show_still_selects_the_host_it_reported(tmp_path: Path) -> None:
    """A host printed before the probe failed still matches the pinned channel."""
    result = run_preflight_against(
        tmp_path, installed_line(pinned_channel()), StubOptions(show_exit_status=1)
    )

    assert result.returncode == 0, result.stderr
    assert "using the host it reported" in result.stderr, result.stderr
    assert "is not installed" not in result.stderr, result.stderr


def test_the_rustup_probes_cannot_auto_install(tmp_path: Path) -> None:
    """A probe may not provision the toolchain it exists to find missing."""
    env_log = tmp_path / "rustup-auto-install"
    result = run_preflight_against(
        tmp_path, installed_line(pinned_channel()), StubOptions(env_log=env_log)
    )

    assert result.returncode == 0, result.stderr
    recorded = env_log.read_text("utf-8").splitlines()
    assert recorded == ["0", "0"], recorded


def test_the_stub_records_into_a_directory_holding_an_apostrophe(
    tmp_path: Path,
) -> None:
    """A quote in the scratch path cannot break the shell the stub is written in."""
    scratch = tmp_path / "O'Brien"
    scratch.mkdir()
    argv_log = scratch / "rustup-argv"
    pwd_log = scratch / "rustup-pwd"
    env_log = scratch / "rustup-auto-install"
    options = StubOptions(argv_log=argv_log, pwd_log=pwd_log, env_log=env_log)

    result = run_preflight_against(scratch, installed_line(pinned_channel()), options)

    assert result.returncode == 0, result.stderr
    recorded = argv_log.read_text("utf-8").splitlines()
    commands = [line.split() for line in recorded]
    assert commands == [["show"], ["toolchain", "list"]], commands
    directories = pwd_log.read_text("utf-8").splitlines()
    assert len(directories) == 2, directories
    auto_install = env_log.read_text("utf-8").splitlines()
    assert auto_install == ["0", "0"], auto_install
