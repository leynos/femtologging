"""Prove production builds withhold the test-only API boundaries.

A release wheel and a default-feature Rust consumer must both be unable to see
the helpers that exist only for tests. The wheel probe inspects the imported
native module, the ``FemtoLogger`` type and the shipped type stub. Three Rust
probes compile tiny consumers against the default features: one control that
names only the production API, and one each for ``TestConfig`` and
``with_writer_for_test``. The control must compile; each negative probe has a
single expected seam, so one resolution error cannot mask the other.
"""

from __future__ import annotations

import json
import os
import pathlib as pth
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] local cargo and wheel probes.
import typing as typ

import pytest

from tests.make_contract_helpers import repo_root
from tests.maturin_compat import (
    build_native_wheel_artifact,
    install_wheel_into_fresh_venv,
    toolchain_available,
)

_FIXTURE_ROOT = pth.Path(__file__).with_name("data") / "rust_api_boundary"
# The negative probe may only fail because a name is unresolved or an item is
# missing. Any other diagnostic means the probe itself is wrong.
_ALLOWED_NEGATIVE_ERROR_CODES = frozenset({"E0412", "E0432", "E0433", "E0599"})
_PROBE_MANIFEST = """\
[package]
name = "api_boundary_{name}"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
_femtologging_rs = {{ package = "femtologging_rs", path = "{dependency}" }}
"""


def _prepare_probe(fixture_name: str, workspace: pth.Path) -> pth.Path:
    """Copy one Rust probe into *workspace* and give it a standalone manifest.

    Parameters
    ----------
    fixture_name:
        Directory name under ``tests/data/rust_api_boundary``.
    workspace:
        Temporary directory that receives the copied probe.

    Returns
    -------
    pathlib.Path
        Path to the generated manifest.
    """
    probe_dir = workspace / fixture_name
    shutil.copytree(_FIXTURE_ROOT / fixture_name, probe_dir)
    dependency = (repo_root() / "rust_extension").resolve()
    manifest = probe_dir / "Cargo.toml"
    manifest.write_text(
        _PROBE_MANIFEST.format(
            name=fixture_name,
            dependency=dependency.as_posix(),
        ),
        encoding="utf-8",
    )
    shutil.copy2(dependency / "Cargo.lock", probe_dir / "Cargo.lock")
    # The probe lives outside the repository, so copy the pinned toolchain file
    # with it; otherwise rustup would silently fall back to the default channel
    # and the check would stop matching the project's compiler.
    shutil.copy2(repo_root() / "rust-toolchain.toml", probe_dir / "rust-toolchain.toml")
    return manifest


def _cargo_check(manifest: pth.Path) -> subprocess.CompletedProcess[str]:
    """Run ``cargo check`` for one probe against the default features.

    The check resolves only from the local cargo cache. The probe copies the
    repository lockfile, and the test suite must not hold the shared
    package-cache lock while a nested Cargo build queries the registry.

    Parameters
    ----------
    manifest:
        Manifest of the copied probe crate.

    Returns
    -------
    subprocess.CompletedProcess[str]
        Completed check whose standard output carries JSON compiler messages.
    """
    cargo = shutil.which("cargo")
    if cargo is None:
        pytest.skip("cargo is required to check the Rust API boundary")
    target_dir = repo_root() / "rust_extension" / "target"
    environment = {
        **os.environ,
        "CARGO_TARGET_DIR": str(target_dir),
        # Match the Makefile: a probe that silently accepted a newer ABI would
        # stop proving what the project's own builds do.
        "PYO3_USE_ABI3_FORWARD_COMPATIBILITY": "0",
    }
    # ruff: ignore[subprocess-without-shell-equals-true] the command vector is
    # built from repository-local paths and the resolved cargo binary only.
    return subprocess.run(
        [
            cargo,
            "check",
            "--offline",
            "--manifest-path",
            str(manifest),
            "--message-format=json",
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=1800,
        env=environment,
        cwd=manifest.parent,
    )


def _compiler_errors(stdout: str) -> list[tuple[str, str]]:
    """Return ``(error code, rendered text)`` pairs from cargo's JSON output.

    Parameters
    ----------
    stdout:
        Standard output produced by ``cargo check --message-format=json``.

    Returns
    -------
    list[tuple[str, str]]
        Error diagnostics in emission order. Non-error messages are dropped.
    """
    errors: list[tuple[str, str]] = []
    for line in stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if message.get("reason") != "compiler-message":
            continue
        diagnostic = message.get("message", {})
        if diagnostic.get("level") != "error":
            continue
        code: typ.Any = diagnostic.get("code")
        error_code = "" if code is None else str(code.get("code", ""))
        errors.append((error_code, str(diagnostic.get("rendered", ""))))
    return errors


@pytest.fixture(scope="module", name="release_wheel")
def fixture_release_wheel(tmp_path_factory: pytest.TempPathFactory) -> pth.Path:
    """Build the production wheel once for the API-boundary checks."""
    if not toolchain_available():
        pytest.skip("Rust toolchain and maturin are required to build a wheel")
    return build_native_wheel_artifact(
        repo_root(), tmp_path_factory.mktemp("api-boundary-wheel")
    )


@pytest.mark.timeout(3620)
def test_release_wheel_omits_test_only_api(
    release_wheel: pth.Path, tmp_path: pth.Path
) -> None:
    """A production wheel imports without test-only Python or stub surface."""
    python = install_wheel_into_fresh_venv(tmp_path / "venv", release_wheel)
    probe = repo_root() / "scripts/release_wheel_probe.py"
    # ruff: ignore[subprocess-without-shell-equals-true] fixed local probe and
    # interpreter paths.
    result = subprocess.run(
        [str(python), "-I", str(probe)],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
        cwd=tmp_path,
    )
    assert result.returncode == 0, (
        f"release wheel probe failed:\n{result.stdout}{result.stderr}"
    )
    assert "release wheel probe passed" in result.stdout, (
        "the probe must confirm the production surface"
    )


@pytest.mark.timeout(1800)
def test_default_feature_build_accepts_production_api(tmp_path: pth.Path) -> None:
    """A default-feature consumer can compile against the production API."""
    manifest = _prepare_probe("production_api", tmp_path)
    result = _cargo_check(manifest)
    assert result.returncode == 0, (
        f"production API probe failed to compile:\n{result.stdout}{result.stderr}"
    )


def _assert_negative_probe_fails_on(
    result: subprocess.CompletedProcess[str], symbol: str
) -> None:
    """Require *result* to fail only because *symbol* is unavailable.

    Parameters
    ----------
    result:
        Completed ``cargo check`` for a negative probe.
    symbol:
        The test-only item the probe names and the diagnostics must mention.

    Examples
    --------
    >>> diagnostic = {
    ...     "reason": "compiler-message",
    ...     "message": {
    ...         "level": "error",
    ...         "code": {"code": "E0432"},
    ...         "rendered": "no `TestConfig` in the root",
    ...     },
    ... }
    >>> result = subprocess.CompletedProcess([], 1, json.dumps(diagnostic), "")
    >>> _assert_negative_probe_fails_on(result, "TestConfig")
    """
    errors = _compiler_errors(result.stdout)
    assert errors, f"cargo reported no compiler errors:\n{result.stderr}"
    unexpected = {code for code, _ in errors} - _ALLOWED_NEGATIVE_ERROR_CODES
    assert not unexpected, (
        "the probe failed for reasons other than unresolved names: "
        f"{sorted(unexpected)}"
    )
    rendered = "\n".join(text for _, text in errors)
    assert symbol in rendered, f"the probe must fail on `{symbol}`"


@pytest.mark.timeout(1800)
def test_default_feature_build_rejects_test_only_api(tmp_path: pth.Path) -> None:
    """A default-feature consumer cannot name the test-only Rust seams."""
    manifest = _prepare_probe("test_only_api", tmp_path)
    result = _cargo_check(manifest)
    assert result.returncode != 0, (
        "the test-only probe compiled under default features, so a test seam "
        "leaked into the production API"
    )
    _assert_negative_probe_fails_on(result, "TestConfig")


@pytest.mark.timeout(1800)
def test_default_feature_build_rejects_test_only_constructor(
    tmp_path: pth.Path,
) -> None:
    """A default-feature consumer cannot call the test-only constructor."""
    manifest = _prepare_probe("test_only_constructor", tmp_path)
    result = _cargo_check(manifest)
    assert result.returncode != 0, (
        "the test-only constructor probe compiled under default features, so "
        "the constructor leaked into the production API"
    )
    _assert_negative_probe_fails_on(result, "with_writer_for_test")
