"""Unit tests for femtologging.file_config path normalization."""

from __future__ import annotations

import typing as typ
from os import PathLike, fsencode
from pathlib import Path

import pytest
from hypothesis import given
from hypothesis import strategies as st

from femtologging.file_config import _normalize_path

if typ.TYPE_CHECKING:
    import collections.abc as cabc

# Every spelling ``_normalize_path`` accepts for a single filesystem path.
type PathInput = str | bytes | PathLike[str] | PathLike[bytes]
# Builds one such spelling from a source path, so the tests can sweep them.
type PathRepresentation = cabc.Callable[[Path], PathInput]


class _FixedPathLike[AnyStr: (str, bytes)](PathLike[AnyStr]):
    """``PathLike`` returning a fixed ``str`` or ``bytes`` value."""

    def __init__(self, value: AnyStr) -> None:
        self._value = value

    @typ.override
    def __fspath__(self) -> AnyStr:
        return self._value


# Each representation must normalize to the same string as the source path.
PATH_REPRESENTATIONS = [
    pytest.param(str, id="str"),
    pytest.param(lambda path: path, id="path"),
    pytest.param(lambda path: fsencode(str(path)), id="bytes"),
    pytest.param(lambda path: _FixedPathLike(str(path)), id="pathlike-str"),
    pytest.param(
        lambda path: _FixedPathLike(fsencode(str(path))),
        id="pathlike-bytes",
    ),
]

# Path segments that survive ``Path`` normalization unchanged, so the
# generated property can compare against the source path directly.
_PATH_SEGMENTS = st.text(
    alphabet=st.characters(codec="utf-8", exclude_characters="/\x00"),
    min_size=1,
    max_size=12,
).filter(lambda segment: segment not in {".", ".."})


@pytest.mark.parametrize("build_input", PATH_REPRESENTATIONS)
@pytest.mark.parametrize(
    "filename",
    [
        pytest.param("config.ini", id="ascii"),
        pytest.param("umlaut-ü.ini", id="non-ascii"),
    ],
)
def test_normalize_path_accepts_every_representation(
    tmp_path: Path,
    build_input: PathRepresentation,
    filename: str,
) -> None:
    """Absolute paths normalize identically however they are spelled."""
    path = tmp_path / filename

    normalized = _normalize_path(build_input(path))

    assert normalized == str(path), (
        f"_normalize_path must map every spelling of {path} to its string form, "
        f"but the {filename!r} case produced {normalized!r}"
    )


@pytest.mark.parametrize("build_input", PATH_REPRESENTATIONS)
def test_normalize_path_keeps_relative_paths_relative(
    build_input: PathRepresentation,
) -> None:
    """Relative paths must not be resolved against the working directory."""
    path = Path("relative/config.ini")

    normalized = _normalize_path(build_input(path))

    assert normalized == str(path), (
        "_normalize_path must leave relative paths relative, but it returned "
        f"{normalized!r} for {path}"
    )


@given(segments=st.lists(_PATH_SEGMENTS, min_size=1, max_size=4))
def test_normalize_path_is_representation_agnostic(segments: list[str]) -> None:
    """All accepted spellings of one path normalize to the same string.

    This is the invariant the parametrized cases sample: ``_normalize_path``
    is a decoder, so the input representation must never affect its result.
    """
    path = Path(*segments)
    expected = str(path)

    normalized = {
        "str": _normalize_path(expected),
        "path": _normalize_path(path),
        "bytes": _normalize_path(fsencode(expected)),
        "pathlike-str": _normalize_path(_FixedPathLike(expected)),
        "pathlike-bytes": _normalize_path(_FixedPathLike(fsencode(expected))),
    }

    assert set(normalized.values()) == {expected}, (
        f"every representation of {expected!r} must normalize identically, "
        f"but the spellings disagreed: {normalized}"
    )
