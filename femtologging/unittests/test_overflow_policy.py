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


def test_timeout_factory_large_value() -> None:
    """Timeout helper correctly handles very large timeout values."""
    large_value = 2**32
    policy = OverflowPolicy.timeout(large_value)
    assert repr(policy) == f"OverflowPolicy.timeout({large_value})"


def test_timeout_factory_rejects_zero() -> None:
    """Timeout helper rejects zero values to mirror builder validation."""
    with pytest.raises(ValueError, match="timeout must be greater than zero"):
        OverflowPolicy.timeout(0)


def test_factories_support_equality() -> None:
    """Factory helpers produce comparable policy objects for Python usage."""
    drop_a = OverflowPolicy.drop()
    drop_b = OverflowPolicy.drop()
    block = OverflowPolicy.block()

    assert drop_a == drop_b
    assert drop_a != block
    assert OverflowPolicy.timeout(125) == OverflowPolicy.timeout(125)
    assert OverflowPolicy.timeout(125) != OverflowPolicy.timeout(250)


def test_hash_consistency() -> None:
    """Policies usable as dictionary keys expose stable hash values."""
    drop_hash = hash(OverflowPolicy.drop())
    block_hash = hash(OverflowPolicy.block())

    assert drop_hash == hash(OverflowPolicy.drop())
    assert block_hash == hash(OverflowPolicy.block())
    assert drop_hash != block_hash
    assert hash(OverflowPolicy.timeout(200)) == hash(OverflowPolicy.timeout(200))


def test_string_representation_matches_repr() -> None:
    """String conversion mirrors the repr output for readability."""
    policy = OverflowPolicy.timeout(750)
    assert str(policy) == repr(policy)
