from __future__ import annotations

import pytest

from femtologging import OverflowPolicy

"""Unit tests for :class:`OverflowPolicy`."""


def test_drop_factory_repr() -> None:
    """Factory helpers return descriptive representations."""
    policy = OverflowPolicy.drop()
    assert repr(policy) == "OverflowPolicy.drop()"


def test_block_factory_repr() -> None:
    """The block helper mirrors the drop helper semantics."""
    policy = OverflowPolicy.block()
    assert repr(policy) == "OverflowPolicy.block()"


def test_timeout_factory_repr() -> None:
    """Timeout helper encodes the duration in milliseconds."""
    policy = OverflowPolicy.timeout(250)
    assert repr(policy) == "OverflowPolicy.timeout(250)"


def test_timeout_factory_rejects_zero() -> None:
    """Timeout helper rejects zero values to mirror builder validation."""
    with pytest.raises(ValueError, match="timeout must be greater than zero"):
        OverflowPolicy.timeout(0)
