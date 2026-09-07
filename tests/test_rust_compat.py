"""Tests for optional Rust compatibility hooks."""

from __future__ import annotations

import pytest

from femtologging._rust_compat import _has_timed_rotation_test_util_support


def _noop() -> None:
    """Provide a simple callable for hook availability tests."""


@pytest.mark.parametrize(
    ("setter", "clearer", "is_supported"),
    [
        pytest.param(_noop, _noop, True, id="both-hooks-callable"),
        pytest.param(_noop, None, False, id="clearer-missing"),
        pytest.param(None, _noop, False, id="setter-missing"),
        pytest.param(None, None, False, id="both-hooks-missing"),
        # The extension exposes attributes, not just presence, so a non-callable
        # attribute must be rejected as firmly as a missing one.
        pytest.param(_noop, "not callable", False, id="clearer-not-callable"),
        pytest.param(0, _noop, False, id="setter-not-callable"),
    ],
)
def test_has_timed_rotation_test_util_support(
    setter: object,
    clearer: object,
    *,
    is_supported: bool,
) -> None:
    """Timed rotation test support requires both hooks to be callable."""
    assert _has_timed_rotation_test_util_support(setter, clearer) is is_supported, (
        "timed rotation test hook support should be reported only when both "
        f"hooks are callable: setter={setter!r} clearer={clearer!r} "
        f"expected={is_supported}"
    )
