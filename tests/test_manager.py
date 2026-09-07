"""Unit tests for the logger manager helpers."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import get_logger, reset_manager

if typ.TYPE_CHECKING:
    import collections.abc as cabc

_INVALID_NAME_MATCH = (
    "logger name cannot be empty, start or end with '\\.', or contain consecutive dots"
)


@pytest.fixture(autouse=True)
def _clean_manager() -> cabc.Iterator[None]:
    """Give every test a manager registry free of loggers from prior tests."""
    reset_manager()
    yield
    reset_manager()


def _assert_parent(name: str, expected_parent: str | None) -> None:
    """Assert the logger called ``name`` reports ``expected_parent``."""
    parent = get_logger(name).parent
    assert parent == expected_parent, (
        f"logger {name!r} should derive parent {expected_parent!r} from its "
        f"dotted name, got {parent!r}"
    )


def test_get_logger_singleton() -> None:
    """Requesting the same logger name should return the same instance."""
    first = get_logger("app.core")
    second = get_logger("app.core")
    assert first is second, (
        "the manager must cache loggers by name so repeated lookups share state"
    )


def test_get_logger_different_names() -> None:
    """Different logger names should produce distinct instances."""
    first = get_logger("first")
    second = get_logger("second")
    assert first is not second, (
        "distinct logger names must map to distinct logger instances"
    )


@pytest.mark.parametrize(
    ("name", "expected_parent"),
    [
        pytest.param("a.b.c", "a.b", id="grandchild_parents_to_child"),
        pytest.param("a.b", "a", id="child_parents_to_top_level"),
        pytest.param("a", "root", id="top_level_parents_to_root"),
        pytest.param("root", None, id="root_has_no_parent"),
    ],
)
def test_get_logger_parents(name: str, expected_parent: str | None) -> None:
    """Logger names should derive parents from dotted notation.

    Each case starts from a clean registry, so the ancestor chain is created
    on demand rather than by a preceding request.
    """
    _assert_parent(name, expected_parent)


def test_get_logger_auto_creates_root() -> None:
    """Creating an undotted child should auto-create the root logger."""
    _assert_parent("child", "root")
    _assert_parent("root", None)


@pytest.mark.parametrize(
    "name",
    [
        pytest.param("", id="empty"),
        pytest.param(".bad", id="leading_dot"),
        pytest.param("bad.", id="trailing_dot"),
        pytest.param("a..b", id="consecutive_dots"),
    ],
)
def test_get_logger_invalid_names(name: str) -> None:
    """Invalid names should raise ValueError."""
    with pytest.raises(ValueError, match=_INVALID_NAME_MATCH):
        get_logger(name)
