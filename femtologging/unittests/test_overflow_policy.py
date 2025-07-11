from __future__ import annotations

from femtologging import OverflowPolicy

"""Unit tests for :class:`OverflowPolicy`."""


def test_enum_values() -> None:
    """Enum members expose the expected string values."""
    assert OverflowPolicy.DROP.value == "drop"
    assert OverflowPolicy.BLOCK.value == "block"
    assert OverflowPolicy.TIMEOUT.value == "timeout"


def test_str_representation() -> None:
    """`str()` returns the underlying value for convenience."""
    assert str(OverflowPolicy.DROP) == "drop"
    assert str(OverflowPolicy.BLOCK) == "block"
    assert str(OverflowPolicy.TIMEOUT) == "timeout"
