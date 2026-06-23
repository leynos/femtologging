"""Shared helpers for maturin compatibility and build-output tests."""

from __future__ import annotations

import email.parser
import re
import shutil
import subprocess  # noqa: S404 - tests invoke pinned maturin build commands.
import sys
import typing as typ
import zipfile

if typ.TYPE_CHECKING:
    from pathlib import Path

_DEV_MATURIN_PIN_RE = re.compile(r"maturin\[patchelf\]==(\d+\.\d+\.\d+)")
_BUILD_MATURIN_RE = re.compile(r"maturin>=(\d+\.\d+\.\d+),<2\.0\.0")
_HEAVY_WORKFLOW_PIN_RE = re.compile(r'"maturin==(\d+\.\d+\.\d+)"')
_GENERATOR_RE = re.compile(r"^Generator:\s*maturin\s*\(([^)]+)\)\s*$", re.MULTILINE)
_EXTENSION_MODULE_RE = re.compile(
    r"^femtologging/_femtologging_rs\.cpython-[^/]+\.so$",
)
_DIST_INFO_SUFFIXES: typ.Final[dict[str, str]] = {
    ".dist-info/RECORD": "femtologging-<version>.dist-info/RECORD",
    ".dist-info/METADATA": "femtologging-<version>.dist-info/METADATA",
    ".dist-info/WHEEL": "femtologging-<version>.dist-info/WHEEL",
    ".dist-info/licenses/LICENSE": "femtologging-<version>.dist-info/licenses/LICENSE",
}


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
    >>> read_expected_maturin_version(repo_root())
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
    >>> pins = read_maturin_pins(repo_root())
    >>> sorted(pins)
    ['build-system', 'dev-dependency', 'heavy-tests']

    """
    pyproject = (root / "pyproject.toml").read_text(encoding="utf-8")
    heavy_tests = (root / ".github/workflows/heavy-tests.yml").read_text(
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
        "heavy-tests": _require_pin_match(
            _HEAVY_WORKFLOW_PIN_RE.search(heavy_tests),
            ".github/workflows/heavy-tests.yml",
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
    result = subprocess.run(  # noqa: S603, RUF100
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
    >>> wheel = build_native_wheel_artifact(repo_root(), tmp_path / "wheelhouse")
    >>> wheel.suffix
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
        "--features",
        "python,test-util",
    ]
    subprocess.run(  # noqa: S603 - command list uses trusted paths and pinned maturin.
        command,
        check=True,
        cwd=root,
    )
    wheels = sorted(out_dir.glob("*.whl"))
    if len(wheels) != 1:
        message = f"Expected exactly one wheel in {out_dir}, found {wheels!r}"
        raise AssertionError(message)
    return wheels[0]


def wheel_build_snapshot(whl_path: Path) -> dict[str, typ.Any]:
    """Return normalized wheel metadata and archive layout.

    Parameters
    ----------
    whl_path:
        Path to a wheel archive produced by maturin.

    Returns
    -------
    dict[str, typing.Any]
        Stable metadata suitable for snapshot comparison.

    Examples
    --------
    >>> snapshot = wheel_build_snapshot(wheel_path)
    >>> snapshot["wheel"]["root_is_purelib"]
    'false'

    """
    with zipfile.ZipFile(whl_path) as archive:
        entry_names = archive.namelist()
        wheel_name = _locate_dist_info_wheel(entry_names)
        metadata_name = wheel_name.replace("/WHEEL", "/METADATA")
        wheel_payload = archive.read(wheel_name).decode("utf-8")
        metadata_payload = archive.read(metadata_name).decode("utf-8")
    generator, root_is_purelib = _parse_wheel_header(wheel_payload, whl_path)
    return {
        "generator": generator,
        "metadata": _parse_metadata(metadata_payload),
        "wheel": {
            "root_is_purelib": root_is_purelib,
            "tag": "<platform-tag>",
        },
        "entries": sorted(_normalize_wheel_entry(name) for name in entry_names),
    }


def _parse_metadata(raw_metadata: str) -> dict[str, typ.Any]:
    """Parse wheel metadata headers into stable values."""
    headers = email.parser.Parser().parsestr(raw_metadata)
    return {
        "name": headers.get("Name"),
        "version": headers.get("Version"),
        "requires_python": headers.get("Requires-Python"),
        "requires_dist": sorted(headers.get_all("Requires-Dist", [])),
        "classifiers": sorted(headers.get_all("Classifier", [])),
    }


def _normalize_wheel_entry(name: str) -> str:
    """Normalize platform-specific wheel archive entry names."""
    if _EXTENSION_MODULE_RE.match(name):
        return "femtologging/_femtologging_rs.cpython-<platform>.so"
    if "/sboms/" in name:
        return "femtologging-<version>.dist-info/sboms/<sbom>.cyclonedx.json"
    for suffix, normalized in _DIST_INFO_SUFFIXES.items():
        if name.endswith(suffix):
            return normalized
    return name


def _locate_dist_info_wheel(entry_names: list[str]) -> str:
    """Return the wheel metadata entry from an archive name list."""
    wheel_name = next(
        (name for name in entry_names if name.endswith(".dist-info/WHEEL")),
        None,
    )
    if wheel_name is None:
        message = "wheel is missing .dist-info/WHEEL metadata"
        raise AssertionError(message)
    return wheel_name


def _parse_wheel_header(wheel_payload: str, whl_path: Path) -> tuple[str, str]:
    """Extract the maturin generator string and purelib flag."""
    generator_match = _GENERATOR_RE.search(wheel_payload)
    if generator_match is None:
        message = f"Could not parse maturin generator from WHEEL metadata: {whl_path}"
        raise AssertionError(message)
    root_is_purelib = next(
        (
            line.removeprefix("Root-Is-Purelib: ")
            for line in wheel_payload.splitlines()
            if line.startswith("Root-Is-Purelib:")
        ),
        None,
    )
    if root_is_purelib is None:
        message = "wheel is missing Root-Is-Purelib metadata"
        raise AssertionError(message)
    return generator_match.group(1), root_is_purelib
