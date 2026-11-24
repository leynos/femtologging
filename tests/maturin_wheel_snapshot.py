"""Wheel-archive snapshot helpers for release-build tests.

These helpers read a built wheel's ``.dist-info`` metadata and archive layout
and normalize the platform-specific parts, so a snapshot can assert the
generator, package metadata, and entry set without pinning a platform tag.
They live apart from :mod:`tests.maturin_compat` to keep each helper module
below the repository's file-length limit.
"""

from __future__ import annotations

import email.parser
import re
import typing as typ
import zipfile

if typ.TYPE_CHECKING:
    from pathlib import Path

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
    Given a wheel path, the helper returns stable metadata for snapshot tests:

    >>> snapshot = wheel_build_snapshot(whl_path)  # doctest: +SKIP
    >>> snapshot["wheel"]["root_is_purelib"]  # doctest: +SKIP
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
