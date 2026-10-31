"""Contract tests for the Rust build standard and the dev-build footprint.

The Concordat ``rust-build-defaults`` and ``rust-dev-build-footprint`` rules ask
for the ``mold`` linker as the Linux default, a development profile that keeps
no incremental state or debug information, Loom steps that restate the linker
flag they would otherwise lose, and a Whitaker route that takes none of the
development standard. These tests hold each fact, then prove the profile
settings and both documented overrides at the Cargo boundary by building a
one-file crate under this repository's own Cargo configuration.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] the contract runs a fixed make and cargo
import tomllib
import typing as typ
from pathlib import Path

import pytest
import yaml

ROOT: typ.Final[Path] = Path(__file__).resolve().parents[1]
MOLD_FLAG: typ.Final[str] = "-Clink-arg=-fuse-ld=mold"
LINUX_TABLE: typ.Final[str] = 'cfg(target_os = "linux")'
RUSTC_LINE: typ.Final[re.Pattern[str]] = re.compile(
    r"Running `.*--crate-name footprint .*`"
)
WRAPPER_VARIABLES: typ.Final[tuple[str, ...]] = (
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "RUSTFLAGS",
    "CARGO_INCREMENTAL",
    "CARGO_PROFILE_DEV_DEBUG",
    "CARGO_PROFILE_DEV_INCREMENTAL",
)


def _load(relative: str) -> dict[str, typ.Any]:
    """Return a parsed TOML file of the repository.

    Parameters
    ----------
    relative
        Path of the TOML file relative to the repository root.

    Returns
    -------
    dict[str, typing.Any]
        The parsed document.
    """
    return tomllib.loads((ROOT / relative).read_text("utf-8"))


def _is_zero_debug(debug: object) -> bool:
    """Accept the exact integer ``0`` or the boolean ``false``.

    Python equality would also accept ``0.0``, which Cargo rejects.

    Parameters
    ----------
    debug
        The ``debug`` value read from a profile.

    Returns
    -------
    bool
        Whether the value disables debug information.
    """
    is_zero_integer = (
        isinstance(debug, int) and not isinstance(debug, bool) and debug == 0
    )
    return debug is False or is_zero_integer


def test_the_linux_table_carries_mold_and_nothing_else_names_it() -> None:
    """Only the Linux-only target table names the mold linker flag."""
    config = _load(".cargo/config.toml")
    targets = config.get("target", {})
    assert MOLD_FLAG in targets.get(LINUX_TABLE, {}).get("rustflags", []), (
        f"[target.'{LINUX_TABLE}'] must carry {MOLD_FLAG}"
    )
    other_sources = [
        config.get("build", {}).get("rustflags", []),
        *(
            table.get("rustflags", [])
            for name, table in targets.items()
            if name != LINUX_TABLE
        ),
    ]
    assert not any(
        "mold" in str(flag) for source in other_sources for flag in source
    ), "mold ships for Linux only, so no other rustflags source may name it"
    assert "unstable" not in config, "a stable pin cannot read an [unstable] table"


def test_the_pin_is_stable() -> None:
    """The native metadata key and the parallel frontend need a nightly pin."""
    channel = _load("rust-toolchain.toml")["toolchain"]["channel"]
    assert not channel.startswith("nightly"), f"expected a stable pin, found {channel}"


def test_development_profile_keeps_the_build_footprint_settings() -> None:
    """The extension's dev profile sets ``incremental = false`` and ``debug = 0``."""
    dev = _load("rust_extension/Cargo.toml").get("profile", {}).get("dev", {})
    assert dev.get("incremental") is False, "profile.dev must set incremental = false"
    assert _is_zero_debug(dev.get("debug")), "profile.dev must set debug = 0 or false"


def _heavy_steps() -> list[dict[str, typ.Any]]:
    """Return the steps of the scheduled heavy-tests job."""
    document = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "heavy-tests.yml").read_text("utf-8")
    )
    return list(document["jobs"]["heavy"]["steps"])


def test_the_loom_steps_restate_the_linker_flag() -> None:
    """Each step that assigns ``--cfg loom`` also carries the mold flag."""
    loom_steps = [
        step for step in _heavy_steps() if "--cfg loom" in str(step.get("run", ""))
    ]
    assert len(loom_steps) == 3, f"expected three Loom steps, found {len(loom_steps)}"
    for step in loom_steps:
        for line in str(step["run"]).splitlines():
            if "--cfg loom" in line:
                assert MOLD_FLAG.replace("-Clink-arg", "-C link-arg") in line, (
                    f"{step.get('name')}: RUSTFLAGS must restate the mold flag: {line}"
                )


def test_the_heavy_job_installs_mold_before_it_compiles() -> None:
    """The heavy job installs mold through setup-rust, ahead of every compile."""
    steps = _heavy_steps()
    install = next(
        (
            index
            for index, step in enumerate(steps)
            if "setup-rust" in str(step.get("uses", ""))
        ),
        None,
    )
    assert install is not None, "the heavy job must install Rust through setup-rust"
    settings = steps[install].get("with", {})
    assert str(settings.get("install-mold")) == "true", "install-mold must be true"
    exports_nothing = "rustflags" in settings and not settings["rustflags"]
    assert exports_nothing, "setup-rust must export no RUSTFLAGS"
    first_compile = next(
        index
        for index, step in enumerate(steps)
        if re.search(r"\b(cargo|maturin)\b", str(step.get("run", "")))
    )
    assert install < first_compile, "mold must be installed before the first compile"


def test_the_whitaker_recipe_is_isolated_from_the_development_flags() -> None:
    """``make -n lint-rust`` runs Whitaker under a literal warning-only RUSTFLAGS."""
    make = shutil.which("make")
    if make is None:
        pytest.skip("make is not installed")
    # ruff: ignore[subprocess-without-shell-equals-true] the argument vector is
    # fixed here and the executable is resolved above
    result = subprocess.run(
        [make, "-n", "-B", "lint-rust"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, f"make -n lint-rust failed:\n{result.stderr}"
    commands = [line for line in result.stdout.splitlines() if "whitaker" in line]
    assert len(commands) == 1, f"expected one Whitaker command, found {commands}"
    command = commands[0]
    assert re.match(r'^RUSTFLAGS="-D warnings" ', command), (
        f"Whitaker must run under a literal RUSTFLAGS boundary: {command}"
    )
    for flag in ("-Zthreads", "fuse-ld=mold", "codegen-backend", "${RUSTFLAGS"):
        assert flag not in command, f"{flag} must not reach the Whitaker route"


def _dev_profile_text() -> str:
    """Return the extension's ``[profile.dev]`` table text, header included."""
    manifest = (ROOT / "rust_extension" / "Cargo.toml").read_text("utf-8")
    match = re.search(r"^\[profile\.dev\]\n(?:(?!\[).*\n?)*", manifest, re.MULTILINE)
    assert match is not None, "rust_extension/Cargo.toml must carry [profile.dev]"
    return match.group(0)


def _compile_fixture(tmp_path: Path, extra_env: dict[str, str] | None = None) -> str:
    """Build the fixture crate and return the rustc command line Cargo ran.

    The fixture holds the repository's Cargo configuration and the extension's
    ``[profile.dev]`` table, and builds under the repository's pinned channel
    with every wrapper and profile override removed from the environment and
    from any host-level Cargo configuration, so only the repository's own
    settings and the override under test apply.

    Parameters
    ----------
    tmp_path
        Scratch directory for the fixture crate and its target directory.
    extra_env
        Environment variables to add, such as a documented override.

    Returns
    -------
    str
        The ``Running`` line Cargo printed for the fixture crate.
    """
    channel = _load("rust-toolchain.toml")["toolchain"]["channel"]
    crate = tmp_path / "footprint"
    (crate / "src").mkdir(parents=True)
    (crate / ".cargo").mkdir()
    (crate / "Cargo.toml").write_text(
        '[package]\nname = "footprint"\nversion = "0.0.0"\nedition = "2021"\n\n'
        + _dev_profile_text(),
        "utf-8",
    )
    (crate / "src" / "lib.rs").write_text("pub fn footprint() {}\n", "utf-8")
    shutil.copy(ROOT / ".cargo" / "config.toml", crate / ".cargo" / "config.toml")
    env = {k: v for k, v in os.environ.items() if k not in WRAPPER_VARIABLES}
    env["CARGO_TARGET_DIR"] = str(tmp_path / "target")
    env.update(extra_env or {})
    cargo = shutil.which("cargo")
    assert cargo is not None, "cargo must be on PATH"
    # A host may configure a compiler wrapper in its own Cargo configuration,
    # which removing the environment variables cannot undo; an admission
    # wrapper would then queue this one-file build behind unrelated work and
    # the fixture would measure the host rather than the repository. An empty
    # command-line value disables the configured wrapper, and command-line
    # configuration outranks every file.
    # ruff: ignore[subprocess-without-shell-equals-true] the argument vector is
    # fixed here and the executable is resolved above
    result = subprocess.run(
        [
            cargo,
            f"+{channel}",
            "build",
            "--config",
            'build.rustc-wrapper=""',
            "--config",
            'build.rustc-workspace-wrapper=""',
            "--color",
            "never",
            "--offline",
            "-v",
        ],
        cwd=crate,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, f"cargo build failed:\n{result.stderr}"
    lines = RUSTC_LINE.findall(result.stderr)
    assert lines, f"no rustc command line in Cargo's output:\n{result.stderr}"
    return lines[0]


def test_cargo_builds_by_default_without_incremental_state_or_debug_info(
    tmp_path: Path,
) -> None:
    """A default dev build passes neither ``debuginfo`` nor ``incremental``."""
    command = _compile_fixture(tmp_path)
    assert "-C debuginfo" not in command, f"debug info must be off: {command}"
    assert "-C incremental" not in command, f"incremental must be off: {command}"


def test_the_documented_debug_override_turns_debug_info_back_on(
    tmp_path: Path,
) -> None:
    """``CARGO_PROFILE_DEV_DEBUG=true`` restores debug information."""
    command = _compile_fixture(tmp_path, {"CARGO_PROFILE_DEV_DEBUG": "true"})
    assert "-C debuginfo=2" in command, f"the override must restore debug: {command}"


def test_the_documented_incremental_override_turns_incremental_back_on(
    tmp_path: Path,
) -> None:
    """``CARGO_INCREMENTAL=1`` restores incremental compilation."""
    command = _compile_fixture(tmp_path, {"CARGO_INCREMENTAL": "1"})
    assert "-C incremental=" in command, f"the override must restore it: {command}"


def test_the_ci_build_installs_mold_for_the_nested_trybuild_crates() -> None:
    """The pull-request build installs mold, which trybuild's nested crate needs."""
    document = yaml.safe_load(
        (ROOT / ".github" / "workflows" / "ci.yml").read_text("utf-8")
    )
    steps = document["jobs"]["build-test"]["steps"]
    setup = next(step for step in steps if "setup-rust" in str(step.get("uses", "")))
    assert str(setup.get("with", {}).get("install-mold")) == "true", (
        "ci.yml must install mold: trybuild's nested crate finds .cargo/config.toml"
    )
