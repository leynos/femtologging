"""Unit tests for femtologging.file_config path normalisation."""

from __future__ import annotations

from os import fsencode
from pathlib import Path

from femtologging.file_config import _normalise_path


class _PathLike:
    """Simple os.PathLike implementation for testing."""

    def __init__(self, value: str | bytes):
        self._value = value

    def __fspath__(self) -> str | bytes:  # pragma: no cover - invoked implicitly
        return self._value


def test_normalise_path_accepts_str(tmp_path: Path) -> None:
    path = tmp_path / "config.ini"

    assert _normalise_path(str(path)) == str(path)


def test_normalise_path_accepts_path(tmp_path: Path) -> None:
    path = tmp_path / "config.ini"

    assert _normalise_path(path) == str(path)


def test_normalise_path_accepts_bytes(tmp_path: Path) -> None:
    path = tmp_path / "config.ini"
    as_bytes = fsencode(str(path))

    assert _normalise_path(as_bytes) == str(path)


def test_normalise_path_accepts_pathlike_str(tmp_path: Path) -> None:
    path = tmp_path / "config.ini"
    path_like = _PathLike(str(path))

    assert _normalise_path(path_like) == str(path)


def test_normalise_path_accepts_pathlike_bytes(tmp_path: Path) -> None:
    path = tmp_path / "config.ini"
    path_like = _PathLike(fsencode(str(path)))

    assert _normalise_path(path_like) == str(path)


def test_normalise_path_handles_relative_input() -> None:
    path = Path("relative/config.ini")

    assert _normalise_path(path) == str(path)


def test_normalise_path_decodes_utf8_bytes(tmp_path: Path) -> None:
    path = tmp_path / "umlaut-\u00fc.ini"
    as_bytes = str(path).encode("utf-8")

    assert _normalise_path(as_bytes) == str(path)
