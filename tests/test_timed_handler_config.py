"""Tests for timed rotating handler argument parsing."""

from __future__ import annotations

import pytest

from femtologging._timed_handler_config import parse_timed_args

# Keyword arguments accepted by ``logging.handlers.TimedRotatingFileHandler``
# but deliberately unsupported by femtologging's options object.
STDLIB_ONLY_KEYWORDS = ("encoding", "delay", "errors")


def _timed_kwargs(**extra: object) -> dict[str, object]:
    """Return the minimal valid timed-handler kwargs, merged with *extra*."""
    return {
        "path": "app.log",
        "when": "S",
        "interval": 1,
        "backup_count": 1,
        **extra,
    }


@pytest.mark.parametrize(
    ("args_t", "kwargs_d", "name"),
    [
        pytest.param(
            ("app.log", "S", 1, 1, None),
            {"encoding": None},
            "encoding",
            id="encoding",
        ),
        pytest.param(
            ("app.log", "S", 1, 1, None, False),
            {"delay": False},
            "delay",
            id="delay",
        ),
        pytest.param(
            ("app.log", "S", 1, 1, None, False, False, None, None),
            {"errors": None},
            "errors",
            id="errors",
        ),
    ],
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
        pytest.param(_timed_kwargs(encoding=None), "encoding", id="encoding"),
        pytest.param(_timed_kwargs(delay=False), "delay", id="delay"),
        pytest.param(_timed_kwargs(errors=None), "errors", id="errors"),
    ],
)
def test_parse_timed_args_strips_valid_stdlib_only_kwargs(
    kwargs_d: dict[str, object],
    name: str,
) -> None:
    """Stdlib-only kwargs supplied only as keywords should be stripped."""
    path, options = parse_timed_args((), kwargs_d)

    assert path == "app.log", (
        f"parse_timed_args must return the configured path; got {path!r}"
    )
    assert options is not None, (
        f"a valid {name!r} keyword must still yield TimedHandlerOptions"
    )
    exposed = [keyword for keyword in STDLIB_ONLY_KEYWORDS if hasattr(options, keyword)]
    assert not exposed, (
        "TimedHandlerOptions must not expose stdlib-only attributes, but it "
        f"exposes {exposed}"
    )
    assert name not in kwargs_d, (
        f"parse_timed_args must consume {name!r} from the caller's kwargs, "
        f"leaving {sorted(kwargs_d)}"
    )


@pytest.mark.parametrize(
    ("kwargs_d", "name"),
    [
        pytest.param(_timed_kwargs(encoding="utf-8"), "encoding", id="encoding"),
        pytest.param(_timed_kwargs(delay=True), "delay", id="delay"),
        pytest.param(_timed_kwargs(errors="ignore"), "errors", id="errors"),
    ],
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
