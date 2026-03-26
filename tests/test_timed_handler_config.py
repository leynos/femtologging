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
