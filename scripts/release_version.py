"""Check ``pyproject.toml``'s declared version against a release tag.

The shared Cargo validator checks the extension crate. This standard-library
script checks the Python project version with exact string equality and
reports whether the release is a pre-release.

Examples
--------
>>> is_prerelease("0.2.0-beta1"), is_prerelease("1.2.3+local")
(True, False)
"""

from __future__ import annotations

import os
import re
import tomllib
import typing as typ
from pathlib import Path

if typ.TYPE_CHECKING:
    import collections.abc as cabc

_LABEL = re.compile(r"preview|alpha|beta|post|pre|rev|dev|rc|a|b|c|r")
_PRERELEASE_LABELS = frozenset({
    "preview",
    "alpha",
    "beta",
    "pre",
    "dev",
    "rc",
    "a",
    "b",
    "c",
})


def is_prerelease(version: str) -> bool:
    """Return whether a PEP 440 or SemVer version is a pre-release.

    Parameters
    ----------
    version
        The declared project version.

    Returns
    -------
    bool
        True for pre-release or development labels; local build metadata is
        ignored.

    Examples
    --------
    >>> is_prerelease("0.2.0-beta1")
    True
    >>> is_prerelease("1.2.3.post1")
    False
    """
    public = version.lower().partition("+")[0]
    return any(label in _PRERELEASE_LABELS for label in _LABEL.findall(public))


def main(environ: cabc.Mapping[str, str], root: Path) -> int:
    """Compare the tag and project versions, then write prerelease status."""
    tag_name = environ["TAG_NAME"]
    expected = tag_name.removeprefix("v")
    project_path_value = environ.get("PYPROJECT_PATH")
    project_path = (
        Path(project_path_value)
        if project_path_value is not None
        else root / "pyproject.toml"
    )
    document = tomllib.loads(project_path.read_text(encoding="utf-8"))
    declared = document["project"]["version"]
    if not isinstance(declared, str) or declared != expected:
        print(
            "::error file=pyproject.toml,title=Tag/pyproject.toml mismatch::"
            f"Tag version {expected} does not match project version {declared}"
        )
        return 1

    prerelease = str(is_prerelease(declared)).lower()
    with Path(environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        output.write(f"prerelease={prerelease}\n")
    print(f"Release tag {tag_name} matches pyproject.toml.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(os.environ, Path.cwd()))
