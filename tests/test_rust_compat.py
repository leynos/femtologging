"""Tests for optional Rust compatibility hooks."""

from __future__ import annotations

from femtologging._rust_compat import _has_timed_rotation_test_util_support


def _noop() -> None:
    """Provide a simple callable for hook availability tests."""


def test_has_timed_rotation_test_util_support_requires_both_hooks() -> None:
    """Timed rotation test support should require both setter and clearer."""
    assert _has_timed_rotation_test_util_support(_noop, _noop), (
        "_has_timed_rotation_test_util_support(_noop, _noop) "
        "should report full timed rotation test hook support"
    )
    assert not _has_timed_rotation_test_util_support(_noop, None), (
        "_has_timed_rotation_test_util_support(_noop, None) "
        "should reject a missing clearer hook"
    )
    assert not _has_timed_rotation_test_util_support(None, _noop), (
        "_has_timed_rotation_test_util_support(None, _noop) "
        "should reject a missing setter hook"
    )
