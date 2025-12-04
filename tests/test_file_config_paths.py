"""Unit tests for femtologging.file_config path normalisation."""

from __future__ import annotations

from os import PathLike, fsencode
from pathlib import Path

from femtologging.file_config import _normalise_path


class _StrPathLike(PathLike[str]):
    """Simple ``PathLike`` returning ``str`` for testing."""

    def __init__(self, value: str) -> None:
        self._value = value

    def __fspath__(self) -> str:  # pragma: no cover - invoked implicitly
        return self._value


class _BytesPathLike(PathLike[bytes]):
    """Simple ``PathLike`` returning ``bytes`` for testing."""

    def __init__(self, value: bytes) -> None:
        self._value = value

    def __fspath__(self) -> bytes:  # pragma: no cover - invoked implicitly
        return self._value


def test_normalise_path_accepts_str(tmp_path: Path) -> None:
    """Str input should round-trip via _normalise_path."""
    path = tmp_path / "config.ini"

    assert _normalise_path(str(path)) == str(path)


def test_normalise_path_accepts_path(tmp_path: Path) -> None:
    """Path input should round-trip via _normalise_path."""
    path = tmp_path / "config.ini"

    assert _normalise_path(path) == str(path)


def test_normalise_path_accepts_bytes(tmp_path: Path) -> None:
    """Bytes input should decode using UTF-8."""
    path = tmp_path / "config.ini"
    as_bytes = fsencode(str(path))

    assert _normalise_path(as_bytes) == str(path)


def test_normalise_path_accepts_pathlike_str(tmp_path: Path) -> None:
    """PathLike[str] input should be accepted."""
    path = tmp_path / "config.ini"
    path_like = _StrPathLike(str(path))

    assert _normalise_path(path_like) == str(path)


def test_normalise_path_accepts_pathlike_bytes(tmp_path: Path) -> None:
    """PathLike[bytes] input should be accepted."""
    path = tmp_path / "config.ini"
    path_like = _BytesPathLike(fsencode(str(path)))

    assert _normalise_path(path_like) == str(path)


def test_normalise_path_handles_relative_input() -> None:
    """Relative paths should be returned unchanged as strings."""
    path = Path("relative/config.ini")

    assert _normalise_path(path) == str(path)


def test_normalise_path_decodes_utf8_bytes(tmp_path: Path) -> None:
    """UTF-8 bytes should decode and preserve non-ASCII characters."""
    path = tmp_path / "umlaut-\u00fc.ini"
    as_bytes = str(path).encode("utf-8")

    assert _normalise_path(as_bytes) == str(path)
