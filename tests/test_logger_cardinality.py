"""Bounded-cardinality tests for the logger registry and dispatch pool.

Purpose
-------
Verify the bounds recorded in ADR 007: logger names are rejected past a byte
and segment budget, the registry refuses new names once it is full instead of
growing without limit, and the number of operating-system threads the logging
system uses depends on the machine rather than on how many loggers exist.

Notes
-----
The thread-count case reads ``/proc/self/status`` and is therefore Linux-only;
it is skipped elsewhere rather than asserting a bound it cannot measure.

The registry-cap case registers `MAX_LOGGERS` distinct loggers, which is
deliberately the largest thing these tests do. Before the shared dispatch pool
existed that would have started ten thousand threads -- the defect itself --
so this test could not be written until the pool landed.

Examples
--------
>>> from femtologging import get_logger
>>> get_logger("demo") is not None
True

"""

from __future__ import annotations

import gc
import os
import sys
import time
import typing as typ
from pathlib import Path

import pytest

import femtologging
from femtologging import StreamHandlerBuilder, basicConfig, get_logger, reset_manager

if typ.TYPE_CHECKING:
    import collections.abc as cabc

# Mirrors `MAX_LOGGERS` in `rust_extension/src/manager/registry.rs`.
MAX_LOGGERS = 10_000

# Mirrors `MAX_LOGGER_NAME_BYTES` and `MAX_LOGGER_NAME_SEGMENTS` in
# `rust_extension/src/manager/validation.rs`.
MAX_LOGGER_NAME_BYTES = 255
MAX_LOGGER_NAME_SEGMENTS = 32


@pytest.fixture(autouse=True)
def _clean_registry() -> cabc.Iterator[None]:
    """Start and end each test from an empty registry."""
    reset_manager()
    yield
    reset_manager()
    gc.collect()


def test_a_name_at_the_byte_budget_is_accepted() -> None:
    """The byte budget is inclusive, so a name of exactly the limit fits."""
    name = "a" * MAX_LOGGER_NAME_BYTES

    assert get_logger(name) is not None, (
        f"a name of exactly {MAX_LOGGER_NAME_BYTES} bytes must be accepted"
    )


def test_a_name_one_byte_over_the_budget_is_rejected() -> None:
    """One byte past the limit is refused, so the budget is not off by one."""
    name = "a" * (MAX_LOGGER_NAME_BYTES + 1)

    with pytest.raises(ValueError, match="255"):
        get_logger(name)


def test_a_megabyte_name_is_rejected_quickly() -> None:
    """A hostile name is refused without being scanned.

    The byte check runs before the character and segment checks precisely so
    that a megabyte-long name costs a length comparison rather than a walk
    over a megabyte of attacker-supplied text.
    """
    name = "a" * (1024 * 1024)

    with pytest.raises(ValueError, match="255"):
        get_logger(name)


@pytest.mark.parametrize(
    "control",
    [
        "\x00",
        "\n",
        "\r",
        "\t",
        "\x1b",
        "\x7f",
        # C1 controls and the Unicode separators: `str.splitlines()` breaks on
        # these too, so a ban stated in terms of ASCII would leave the forgery
        # available through a character the consumer still treats as a
        # boundary.
        "\x85",
        "\u2028",
        "\u2029",
    ],
)
def test_control_characters_are_rejected(control: str) -> None:
    """A control character in a name is refused.

    The newline case is the one that matters most: a name is rendered into
    every formatted record, so an embedded newline would let a caller forge a
    record boundary in line-oriented output.
    """
    with pytest.raises(ValueError, match="control"):
        get_logger(f"safe{control}name")


def test_a_rejected_name_cannot_forge_a_record_boundary() -> None:
    """The rejection diagnostic escapes the name it quotes back.

    A rejected name is untrusted by definition, so the message describing the
    rejection must not reintroduce the injection the rule exists to stop.
    """
    with pytest.raises(ValueError, match="control") as caught:
        get_logger("forged\nfake [CRITICAL] injected")

    assert "\n" not in str(caught.value), (
        "the diagnostic must not carry a raw newline from the rejected name"
    )


def test_segment_budget_rejects_a_deep_name() -> None:
    """A name deeper than the segment budget is refused.

    The segments are one byte each so the name stays well inside the byte
    budget. Otherwise the length check would fire first and this test would be
    asserting the wrong rule: the checks run cheapest-first, and "refused for
    some reason" is not what the segment rule is about.
    """
    name = ".".join(["a"] * (MAX_LOGGER_NAME_SEGMENTS + 1))

    assert len(name) < MAX_LOGGER_NAME_BYTES, (
        "this case must exercise the segment rule, not the byte rule"
    )
    with pytest.raises(ValueError, match="segments"):
        get_logger(name)


def test_convenience_name_argument_is_validated_too() -> None:
    """The module-level ``name=`` route is bounded like ``get_logger``.

    This is the route folded into issue #432: a ``name`` argument that looks
    like a parameter accepted unbounded distinct values.
    """
    with pytest.raises(ValueError, match="255"):
        femtologging.info("hello", name="a" * (MAX_LOGGER_NAME_BYTES + 1))

    with pytest.raises(ValueError, match="control"):
        femtologging.debug("hello", name="bad\nname")


def _fill_registry(prefix: str, *, room: int = 0) -> None:
    """Fill the registry to the cap with loggers under *prefix*.

    Registering ``prefix.0`` also registers ``prefix``, so one entry of the
    budget is spent on the shared ancestor. Filling from zero therefore stops
    one short of the cap unless the ancestor is counted.

    ``room`` is the number of slots the caller has already spent on loggers it
    registered before calling this, so the fill still stops exactly at the cap.
    Counting the registry's own size would make the helper depend on a bound it
    is meant to be testing.
    """
    get_logger(prefix)  # the ancestor the numbered names hang below
    for index in range(MAX_LOGGERS - 1 - room):
        get_logger(f"{prefix}.{index}")


def test_registry_refuses_new_names_at_the_cap() -> None:
    """The registry stops growing at the cap and says so.

    The refusal is a ``RuntimeError`` rather than an eviction: a logger owns
    handlers, a level and drop counters that a caller may still be using, so
    displacing one would silently redirect its records.
    """
    _fill_registry("cardinality")

    with pytest.raises(RuntimeError, match=str(MAX_LOGGERS)):
        get_logger("cardinality.one-too-many")


def test_a_refused_chain_leaves_the_registry_unchanged() -> None:
    """A name whose ancestors do not fit is refused before anything is added.

    A dotted name registers its whole chain, so the refusal has to be decided
    for the chain as a unit. Refusing halfway would leave a parent registered
    with no child, and the next attempt would be refused for a different reason
    than the caller saw.

    One slot is deliberately left free. Filling the registry completely would
    make the partial-insert case unreachable — nothing could be inserted before
    the refusal either way — and the test could not tell a whole-chain refusal
    from an implementation that inserted ``fresh`` and then refused
    ``fresh.child``. With one slot free, registering an unrelated name
    afterwards is only possible if the refused chain consumed nothing.
    """
    # One slot stays free: the chain below needs two and must be refused whole.
    _fill_registry("crowded", room=1)

    with pytest.raises(RuntimeError, match=str(MAX_LOGGERS)):
        # Two entries are needed: "fresh" and "fresh.child". Only one slot is
        # free, so the chain is refused and nothing may be inserted.
        get_logger("fresh.child")

    assert get_logger("single") is not None, (
        "the refusal must not have consumed the one free slot"
    )
    assert get_logger("crowded.0") is not None, (
        "the refusal must not have disturbed an existing logger"
    )


def test_names_already_registered_still_resolve_at_the_cap() -> None:
    """Reaching the cap does not break the loggers already in the registry."""
    _fill_registry("existing")

    assert get_logger("existing.0") is get_logger("existing.0"), (
        "an already-registered name must keep resolving to the same logger"
    )
    assert get_logger(f"existing.{MAX_LOGGERS - 2}") is not None, (
        "the last name registered before the cap must still resolve"
    )


def test_root_is_always_available_at_the_cap() -> None:
    """``root`` is exempt from the cap so the bridges keep a fallback target."""
    _fill_registry("filled")

    assert get_logger("root") is not None, (
        "the Rust bridges fall back to root, so it must never be refused"
    )


def test_propagation_still_reaches_root_at_the_cap() -> None:
    """A child's records still reach a root handler once the registry is full.

    Propagation resolves the parent through the registry, and the lookup's
    error is deliberately ignored — a parent that cannot be resolved must not
    turn a log call into an exception. That makes a missing parent *silent*,
    which is why registration has to create the whole chain: at the cap, a
    child whose parent was never registered would stop propagating with no
    diagnostic at all.
    """
    records: list[dict[str, object]] = []

    def capture(record: dict[str, object]) -> str:
        records.append(record)
        return "captured"

    handler = StreamHandlerBuilder.stderr().with_formatter(capture).build()
    basicConfig(level="INFO", force=True, handlers=[handler])

    # A dotted name registers its ancestors too, so this spends two of the
    # budget's slots: "propagating" and "propagating.child".
    child = get_logger("propagating.child")
    _fill_registry("filler", room=2)

    with pytest.raises(RuntimeError, match=str(MAX_LOGGERS)):
        get_logger("filler.one-too-many")

    child.info("still reaches root")
    assert child.flush_handlers(), "the child logger did not flush"
    for _ in range(20):
        if records:
            break
        time.sleep(0.01)

    assert records, (
        "a full registry must not break propagation for a child whose parent "
        "already exists"
    )


@pytest.mark.skipif(
    sys.platform != "linux",
    reason="thread counting reads /proc/self/status, which is Linux-only",
)
def test_thread_count_does_not_track_logger_count() -> None:
    """Logger cardinality no longer implies thread cardinality.

    Before the shared dispatch pool each logger owned a worker thread, so this
    assertion could not hold at any count: registering a thousand loggers
    meant a thousand threads. The pool's size is a function of the machine,
    so the growth here must be small and, crucially, must not scale with the
    number of loggers created.
    """
    baseline = _thread_count()
    registrations = 1_000

    for index in range(registrations):
        get_logger(f"threads.{index}")

    growth = _thread_count() - baseline

    assert growth < registrations, (
        f"creating {registrations} loggers grew the thread count by {growth}; "
        f"logger cardinality must not drive thread cardinality"
    )
    assert growth <= _dispatch_pool_ceiling(), (
        f"creating {registrations} loggers grew the thread count by {growth}, "
        f"beyond the dispatch pool's own ceiling"
    )


def _dispatch_pool_ceiling() -> int:
    """Return the most dispatch threads the pool can start on this machine.

    Mirrors the clamp in `rust_extension/src/logger/dispatcher.rs`: at least
    one shard, at most four, otherwise one per available core. A little slack
    is allowed for threads the interpreter itself creates while the loggers
    are registered.

    Returns
    -------
    int
        The dispatch thread count a logger count cannot exceed, plus slack.

    """
    cores = _available_cores()
    shards = min(max(cores, 1), 4)
    return shards + 4


def _available_cores() -> int:
    """Return the machine's available parallelism, or 0 when unknown."""
    try:
        return len(os.sched_getaffinity(0))
    except (AttributeError, OSError):  # pragma: no cover - non-Linux
        return 0


def _thread_count() -> int:
    """Return this process's thread count from ``/proc/self/status``."""
    for line in Path("/proc/self/status").read_text(encoding="utf-8").splitlines():
        if line.startswith("Threads:"):
            return int(line.split()[1])
    message = "/proc/self/status did not report a Threads line"
    raise AssertionError(message)
