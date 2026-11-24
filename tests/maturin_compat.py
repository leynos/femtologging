"""Shared helpers for maturin compatibility and build-output tests."""

from __future__ import annotations

import re
import shutil
import subprocess  # ruff: ignore[suspicious-subprocess-import] tests shell out to a pinned maturin.
import sys
import tomllib
import typing as typ

if typ.TYPE_CHECKING:
    from pathlib import Path

_DEV_MATURIN_PIN_RE = re.compile(r"maturin\[patchelf\]==(\d+\.\d+\.\d+)")
_BUILD_MATURIN_RE = re.compile(r"maturin>=(\d+\.\d+\.\d+),<2\.0\.0")
_BUILD_WORKFLOW_PIN_RE = re.compile(r'MATURIN_VERSION:\s*"(\d+\.\d+\.\d+)"')


def read_expected_maturin_version(root: Path) -> str:
    """Read the exact maturin version pinned for development.

    Parameters
    ----------
    root:
        Repository root containing ``pyproject.toml``.

    Returns
    -------
    str
        The pinned maturin version.

    Raises
    ------
    AssertionError
        If the maturin dependency pin is missing.

    Examples
    --------
    Given a repository root, the helper returns the development dependency pin:

    >>> read_expected_maturin_version(root)  # doctest: +SKIP
    '1.13.3'

    """
    pyproject = (root / "pyproject.toml").read_text(encoding="utf-8")
    match = _DEV_MATURIN_PIN_RE.search(pyproject)
    if match is None:
        message = "Could not locate maturin dev dependency pin in pyproject.toml"
        raise AssertionError(message)
    return match.group(1)


def _require_pin_match(match: re.Match[str] | None, location: str) -> str:
    """Extract a maturin version from a regex match.

    Parameters
    ----------
    match:
        Regex match object containing the version in group 1.
    location:
        Human-readable source location for error messages.

    Returns
    -------
    str
        The matched maturin version.

    Raises
    ------
    AssertionError
        If ``match`` is ``None``.

    Examples
    --------
    >>> _require_pin_match(re.search("(1.13.3)", "1.13.3"), "example")
    '1.13.3'

    """
    if match is None:
        message = f"Could not locate maturin version pin in {location}"
        raise AssertionError(message)
    return match.group(1)


def _locked_maturin_version(lockfile: Path) -> str:
    """Return the maturin version recorded in a uv lock file.

    Parameters
    ----------
    lockfile:
        Path to the repository's ``uv.lock``.

    Returns
    -------
    str
        The locked maturin version.

    Raises
    ------
    AssertionError
        If the lock file records no maturin package or more than one.
    TypeError
        If the recorded version is not a string.

    Examples
    --------
    >>> _locked_maturin_version(Path("uv.lock"))  # doctest: +SKIP
    '1.13.3'

    """
    locked = tomllib.loads(lockfile.read_text(encoding="utf-8"))
    packages = locked.get("package", [])
    matches = [
        package
        for package in packages
        if isinstance(package, dict) and package.get("name") == "maturin"
    ]
    if len(matches) != 1:
        message = f"Expected one maturin package in {lockfile}, found {len(matches)}"
        raise AssertionError(message)
    version = matches[0].get("version")
    if not isinstance(version, str):
        message = f"maturin in {lockfile} carries no string version: {version!r}"
        raise TypeError(message)
    return version


def read_maturin_pins(root: Path) -> dict[str, str]:
    """Read maturin versions from the synchronized build locations.

    Parameters
    ----------
    root:
        Repository root.

    Returns
    -------
    dict[str, str]
        Mapping of source location to maturin version.

    Examples
    --------
    Given a repository root, the helper returns each synchronized maturin pin:

    >>> pins = read_maturin_pins(root)  # doctest: +SKIP
    >>> sorted(pins)  # doctest: +SKIP
    ['build-system', 'build-wheels', 'dev-dependency', 'uv-lock']

    """
    pyproject = (root / "pyproject.toml").read_text(encoding="utf-8")
    build_wheels = (root / ".github/workflows/build-wheels.yml").read_text(
        encoding="utf-8"
    )
    return {
        "dev-dependency": _require_pin_match(
            _DEV_MATURIN_PIN_RE.search(pyproject),
            "pyproject.toml dev dependency",
        ),
        "build-system": _require_pin_match(
            _BUILD_MATURIN_RE.search(pyproject),
            "pyproject.toml build-system requirement",
        ),
        "uv-lock": _locked_maturin_version(root / "uv.lock"),
        "build-wheels": _require_pin_match(
            _BUILD_WORKFLOW_PIN_RE.search(build_wheels),
            ".github/workflows/build-wheels.yml",
        ),
    }


def toolchain_available() -> bool:
    """Return whether the Rust toolchain and maturin module are available.

    Returns
    -------
    bool
        ``True`` when the test environment can invoke cargo, rustc, and
        ``python -m maturin``.

    Examples
    --------
    >>> isinstance(toolchain_available(), bool)
    True

    """
    if shutil.which("cargo") is None or shutil.which("rustc") is None:
        return False
    # The argument vector is a fixed probe of the running interpreter, so no
    # untrusted input reaches the subprocess.
    result = subprocess.run(
        [sys.executable, "-m", "maturin", "--version"],
        check=False,
        capture_output=True,
    )
    return result.returncode == 0


def build_native_wheel_artifact(root: Path, out_dir: Path) -> Path:
    """Build one native femtologging wheel with maturin.

    Parameters
    ----------
    root:
        Repository root.
    out_dir:
        Directory where maturin should write the wheel.

    Returns
    -------
    Path
        Path to the single built wheel.

    Raises
    ------
    AssertionError
        If maturin does not produce exactly one wheel.

    Examples
    --------
    Given a repository root and an output directory, the helper returns the
    single wheel produced by maturin:

    >>> wheel = build_native_wheel_artifact(root, out_dir)  # doctest: +SKIP
    >>> wheel.suffix  # doctest: +SKIP
    '.whl'

    """
    out_dir.mkdir(parents=True, exist_ok=True)
    command = [
        sys.executable,
        "-m",
        "maturin",
        "build",
        "--release",
        "--out",
        str(out_dir),
        "--manifest-path",
        str(root / "rust_extension/Cargo.toml"),
    ]
    # ruff: ignore[subprocess-without-shell-equals-true] the command vector is
    # built from repository-local paths and the pinned maturin module only.
    subprocess.run(
        command,
        check=True,
        cwd=root,
        timeout=3600,
    )
    wheels = sorted(out_dir.glob("*.whl"))
    if len(wheels) != 1:
        message = f"Expected exactly one wheel in {out_dir}, found {wheels!r}"
        raise AssertionError(message)
    return wheels[0]


def install_wheel_into_fresh_venv(environment: Path, wheel: Path) -> Path:
    """Create a virtual environment and install one wheel into it.

    The wheel is installed without dependencies and without contacting an
    index, so the check exercises exactly the archive built by the release
    packaging configuration.

    Parameters
    ----------
    environment:
        Directory where the virtual environment is created.
    wheel:
        Wheel archive to install.

    Returns
    -------
    Path
        Interpreter inside the new environment.

    Raises
    ------
    AssertionError
        If the wheel cannot be installed.

    Examples
    --------
    >>> python = install_wheel_into_fresh_venv(env, wheel)  # doctest: +SKIP

    """
    # Imported here rather than at module scope: `tests/test_maturin_build.py`
    # must be collectable on an interpreter without the stdlib `venv` module,
    # and that test imports this module before it decides to skip.
    import venv

    venv.EnvBuilder(with_pip=True).create(environment)
    python = environment / (
        "Scripts/python.exe" if sys.platform == "win32" else "bin/python"
    )
    # ruff: ignore[subprocess-without-shell-equals-true] fixed local paths.
    result = subprocess.run(
        [
            str(python),
            "-m",
            "pip",
            "install",
            "--no-deps",
            "--no-index",
            str(wheel),
        ],
        check=False,
        capture_output=True,
        text=True,
        timeout=120,
    )
    if result.returncode != 0:
        message = f"wheel installation failed:\n{result.stdout}{result.stderr}"
        raise AssertionError(message)
    return python
