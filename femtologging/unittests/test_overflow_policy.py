"""Unit tests for :class:`OverflowPolicy` helpers and equality semantics."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import OverflowPolicy

if typ.TYPE_CHECKING:
    import collections.abc as cabc

hypothesis = pytest.importorskip(
    "hypothesis", reason="the timeout round-trip property needs Hypothesis"
)
from hypothesis import (  # ruff: ignore[module-import-not-at-top-of-file] must follow importorskip so collection survives without Hypothesis
    given,
)
from hypothesis import (  # ruff: ignore[module-import-not-at-top-of-file] must follow importorskip so collection survives without Hypothesis
    strategies as st,
)

# ``timeout`` accepts a Rust ``u64`` millisecond count, so the property covers
# the whole accepted domain above the rejected zero.
_U64_MAX = 2**64 - 1


def _assert_repr(policy: OverflowPolicy, expected: str) -> None:
    """Assert a policy's repr round-trips to the factory call that built it."""
    assert repr(policy) == expected, (
        f"OverflowPolicy repr must name the factory call that produced it; "
        f"expected {expected!r}, got {repr(policy)!r}"
    )


@pytest.mark.parametrize(
    ("factory", "expected_repr"),
    [
        pytest.param(OverflowPolicy.drop, "OverflowPolicy.drop()", id="drop"),
        pytest.param(OverflowPolicy.block, "OverflowPolicy.block()", id="block"),
    ],
)
def test_nullary_factory_repr(
    factory: cabc.Callable[[], OverflowPolicy], expected_repr: str
) -> None:
    """The argument-free factory helpers return descriptive representations."""
    _assert_repr(factory(), expected_repr)


@pytest.mark.parametrize(
    "timeout_ms",
    [
        pytest.param(250, id="typical_duration"),
        # 2**32 exceeds u32, guarding the FFI conversion of large durations.
        pytest.param(2**32, id="above_u32_range"),
    ],
)
def test_timeout_factory_repr(timeout_ms: int) -> None:
    """The timeout helper encodes the duration in milliseconds."""
    _assert_repr(
        OverflowPolicy.timeout(timeout_ms), f"OverflowPolicy.timeout({timeout_ms})"
    )


@given(timeout_ms=st.integers(min_value=1, max_value=_U64_MAX))
def test_timeout_repr_round_trips_for_any_accepted_duration(timeout_ms: int) -> None:
    """Every accepted millisecond count appears verbatim in the repr."""
    _assert_repr(
        OverflowPolicy.timeout(timeout_ms), f"OverflowPolicy.timeout({timeout_ms})"
    )


def test_timeout_factory_rejects_zero() -> None:
    """Timeout helper rejects zero values to mirror builder validation."""
    with pytest.raises(ValueError, match="timeout must be greater than zero"):
        OverflowPolicy.timeout(0)


def test_factories_support_equality() -> None:
    """Factory helpers produce comparable policy objects for Python usage."""
    drop_a = OverflowPolicy.drop()
    drop_b = OverflowPolicy.drop()
    block = OverflowPolicy.block()

    assert drop_a == drop_b, (
        "separately constructed drop policies must compare equal by value"
    )
    assert drop_a != block, "distinct policy variants must not compare equal"
    assert OverflowPolicy.timeout(125) == OverflowPolicy.timeout(125), (
        "timeout policies with the same duration must compare equal"
    )
    assert OverflowPolicy.timeout(125) != OverflowPolicy.timeout(250), (
        "timeout policies must discriminate on their duration"
    )


def test_hash_consistency() -> None:
    """Policies usable as dictionary keys expose stable hash values."""
    drop_hash = hash(OverflowPolicy.drop())
    block_hash = hash(OverflowPolicy.block())

    assert drop_hash == hash(OverflowPolicy.drop()), (
        "equal drop policies must hash alike so they work as dictionary keys"
    )
    assert block_hash == hash(OverflowPolicy.block()), (
        "equal block policies must hash alike so they work as dictionary keys"
    )
    assert drop_hash != block_hash, (
        "distinct policy variants should not collide, keeping lookups cheap"
    )
    assert hash(OverflowPolicy.timeout(200)) == hash(OverflowPolicy.timeout(200)), (
        "equal timeout policies must hash alike so they work as dictionary keys"
    )


def test_string_representation_matches_repr() -> None:
    """String conversion mirrors the repr output for readability."""
    policy = OverflowPolicy.timeout(750)
    assert str(policy) == repr(policy), (
        "str() must reuse the repr form so log messages stay unambiguous"
    )
