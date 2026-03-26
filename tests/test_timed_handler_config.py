"""Tests for timed rotating handler argument parsing."""

from __future__ import annotations

import pytest

from femtologging._timed_handler_config import parse_timed_args


@pytest.mark.parametrize(
    ("args_t", "kwargs_d", "name"),
    [
        (("app.log", "S", 1, 1, None), {"encoding": None}, "encoding"),
        (("app.log", "S", 1, 1, None, False), {"delay": False}, "delay"),
        (
            ("app.log", "S", 1, 1, None, False, False, None, None),
            {"errors": None},
            "errors",
        ),
    ],
    ids=["encoding", "delay", "errors"],
)
def test_parse_timed_args_rejects_stdlib_slot_duplicate_keywords(
    args_t: tuple[object, ...],
    kwargs_d: dict[str, object],
    name: str,
) -> None:
    """Stdlib-only slots should still participate in duplicate detection."""
    with pytest.raises(
        TypeError,
        match=(
            rf"duplicate argument: '{name}' provided both positionally "
            r"and as keyword"
        ),
    ):
        parse_timed_args(args_t, kwargs_d)


@pytest.mark.parametrize(
    ("kwargs_d", "name"),
    [
        (
            {
                "path": "app.log",
                "when": "S",
                "interval": 1,
                "backup_count": 1,
                "encoding": None,
            },
            "encoding",
        ),
        (
            {
                "path": "app.log",
                "when": "S",
                "interval": 1,
                "backup_count": 1,
                "delay": False,
            },
            "delay",
        ),
        (
            {
                "path": "app.log",
                "when": "S",
                "interval": 1,
                "backup_count": 1,
                "errors": None,
            },
            "errors",
        ),
    ],
    ids=["encoding", "delay", "errors"],
)
def test_parse_timed_args_strips_valid_stdlib_only_kwargs(
    kwargs_d: dict[str, object],
    name: str,
) -> None:
    """Stdlib-only kwargs supplied only as keywords should be stripped."""
    path, options = parse_timed_args((), kwargs_d)

    assert path == "app.log"
    assert options is not None
    assert name not in kwargs_d


@pytest.mark.parametrize(
    ("kwargs_d", "name"),
    [
        (
            {
                "path": "app.log",
                "when": "S",
                "interval": 1,
                "backup_count": 1,
                "encoding": "utf-8",
            },
            "encoding",
        ),
        (
            {
                "path": "app.log",
                "when": "S",
                "interval": 1,
                "backup_count": 1,
                "delay": True,
            },
            "delay",
        ),
        (
            {
                "path": "app.log",
                "when": "S",
                "interval": 1,
                "backup_count": 1,
                "errors": "ignore",
            },
            "errors",
        ),
    ],
    ids=["encoding", "delay", "errors"],
)
def test_parse_timed_args_rejects_invalid_stdlib_only_kwargs(
    kwargs_d: dict[str, object],
    name: str,
) -> None:
    """Invalid stdlib-only kwargs should still fail validation."""
    with pytest.raises(
        ValueError,
        match=rf"{name} parameter is not supported",
    ):
        parse_timed_args((), kwargs_d)
