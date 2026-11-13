"""Bounded concurrency model for the file-handler lifecycle operations.

Issue #441 releases the interpreter lock around ``handle()``, ``flush()``, and
``close()``, so those calls can genuinely overlap. The sequential lifecycle
model in :mod:`tests.test_file_handler_lifecycle_state_machine` cannot observe
that overlap, and the subprocess scenarios pin one interleaving at a time.

This module adds a bounded concurrency model. For each builder, queue capacity
and overflow policy it starts producers, flushers and closers behind one
barrier, then checks the invariants the handler contract promises: every call
returns, shutdown is idempotent, a closed handler rejects later records, a
closed flush reports ``False``, every closer observes the same completion, and
no wait is left in flight. It is a model rather than an exhaustive schedule:
the barrier and the bounded operation counts enumerate a handful of orders per
configuration, which is enough to catch a lost wake-up, a lost shutdown, or a
wait whose outcome is recorded before it returns.
"""

from __future__ import annotations

import dataclasses
import threading
import time
import typing as typ

import pytest

from femtologging import OverflowPolicy

from .test_wait_diagnostics import (
    _file_builder,
    _rotating_builder,
    _timed_rotating_builder,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

    from .test_wait_diagnostics import Handler, HandlerBuilder

#: One builder and the exception its ``handle()`` raises for a rejected record.
_BUILDERS = [
    pytest.param(_file_builder, RuntimeError, id="file"),
    pytest.param(_rotating_builder, RuntimeError, id="rotating"),
    pytest.param(_timed_rotating_builder, ValueError, id="timed_rotating"),
]


@dataclasses.dataclass(frozen=True, slots=True)
class _QueueShape:
    """The queue size and overflow policy one race runs against."""

    capacity: int
    policy: OverflowPolicy


#: The queue behaviours a concurrent caller can meet: a dropped record, an
#: unbounded wait for capacity, and a bounded wait for capacity, each at a
#: capacity of one and of four.
_QUEUE_SHAPES = [
    pytest.param(
        _QueueShape(capacity=capacity, policy=policy),
        id=f"capacity-{capacity}-{policy_name}",
    )
    for capacity in (1, 4)
    for policy, policy_name in (
        (OverflowPolicy.drop(), "drop"),
        (OverflowPolicy.block(), "block"),
        (OverflowPolicy.timeout(25), "timeout"),
    )
]


@dataclasses.dataclass(frozen=True, slots=True)
class _Race:
    """The state every concurrent lifecycle thread in one race shares."""

    start: threading.Barrier
    handler: Handler
    handle_error: type[Exception]
    failures: list[BaseException]


def _join_all(threads: list[threading.Thread], budget: float) -> None:
    """Wait for every thread until one shared deadline expires."""
    deadline = time.monotonic() + budget
    for thread in threads:
        thread.join(timeout=max(0.0, deadline - time.monotonic()))


def _build_handler(
    builder_factory: cabc.Callable[[Path], HandlerBuilder],
    path: Path,
    shape: _QueueShape,
) -> Handler:
    """Build one handler with the queue shape the race is run against."""
    return (
        builder_factory(path)
        .with_capacity(shape.capacity)
        .with_flush_after_records(10_000)
        .with_overflow_policy(shape.policy)
        .build()
    )


def _producer(race: _Race, name: str) -> None:
    """Enqueue until the handler rejects a record or closes."""
    race.start.wait(timeout=5)
    for index in range(8):
        try:
            race.handler.handle("core", "INFO", f"{name}-{index}")
        except race.handle_error:
            break
        except Exception as err:  # ruff: ignore[blind-except] surfaced by the assertion
            race.failures.append(err)
            break


def _flusher(race: _Race) -> None:
    """Flush repeatedly; a closed or timed-out flush is not an error."""
    race.start.wait(timeout=5)
    for _ in range(4):
        try:
            race.handler.flush()
        except Exception as err:  # ruff: ignore[blind-except] surfaced by the assertion
            race.failures.append(err)
            break
        time.sleep(0.001)


def _closer(race: _Race) -> None:
    """Close twice: an idempotent close must return without raising."""
    race.start.wait(timeout=5)
    try:
        race.handler.close()
        race.handler.close()
    except Exception as err:  # ruff: ignore[blind-except] surfaced by the assertion
        race.failures.append(err)


def _start_threads(race: _Race) -> list[threading.Thread]:
    """Start two producers, two flushers and two closers behind the barrier."""
    threads = [
        threading.Thread(
            target=_producer,
            args=(race, f"producer-{index}"),
            daemon=True,
        )
        for index in range(2)
    ]
    threads += [
        threading.Thread(target=_flusher, args=(race,), daemon=True),
        threading.Thread(target=_flusher, args=(race,), daemon=True),
        # Two closers make the close waits overlap rather than running back to
        # back on one thread, so the shared completion signal is exercised.
        threading.Thread(target=_closer, args=(race,), daemon=True),
        threading.Thread(target=_closer, args=(race,), daemon=True),
    ]
    for thread in threads:
        thread.start()
    return threads


def _assert_joined(
    threads: list[threading.Thread], failures: list[BaseException]
) -> None:
    """Fail when a call never returned or a call raised unexpectedly."""
    assert not any(thread.is_alive() for thread in threads), (
        "a concurrent lifecycle call never returned"
    )
    assert failures == [], f"concurrent lifecycle calls raised: {failures!r}"


def _assert_live_diagnostics(handler: Handler) -> None:
    """Check the counters every joined race must have produced."""
    diagnostics = handler.wait_diagnostics()
    assert diagnostics["waits_in_flight"] == 0, (
        f"every joined wait must leave the gauge: {diagnostics!r}"
    )
    # Seeding one record before the race proves the handler was live even when
    # a closer wins the barrier.
    assert diagnostics["handle_accepted"] >= 1, (
        f"the seed record must stay accepted: {diagnostics!r}"
    )
    # Both closers wait on the same completion signal, and whichever order
    # they arrive in each one records exactly one close outcome.
    close_outcomes = sum(
        diagnostics[key]
        for key in (
            "close_completed",
            "close_timed_out",
            "close_panicked",
            "close_unknown",
        )
    )
    assert close_outcomes >= 2, f"every closer must record an outcome: {diagnostics!r}"


def _assert_post_handoff(handler: Handler, handle_error: type[Exception]) -> None:
    """Check the contract after the handler has been closed and joined."""
    # The handler is closed and every thread has returned, so the post-hand-off
    # contract is deterministic: records are rejected and flushes report False
    # rather than hanging or raising.
    with pytest.raises(handle_error):
        handler.handle("core", "INFO", "after-close")
    assert handler.flush() is False, "a closed handler acknowledged a flush"
    diagnostics = handler.wait_diagnostics()
    assert diagnostics["handle_closed"] >= 1, (
        f"the rejected record must be counted as closed: {diagnostics!r}"
    )
    assert diagnostics["flush_closed"] >= 1, (
        f"the closed flush must be counted as closed: {diagnostics!r}"
    )


@pytest.mark.parametrize(("builder_factory", "handle_error"), _BUILDERS)
@pytest.mark.parametrize("shape", _QUEUE_SHAPES)
def test_concurrent_lifecycle_interleavings(
    tmp_path: Path,
    builder_factory: cabc.Callable[[Path], HandlerBuilder],
    handle_error: type[Exception],
    shape: _QueueShape,
) -> None:
    """Overlapping calls keep every documented lifecycle invariant."""
    handler = _build_handler(builder_factory, tmp_path / "race.log", shape)
    handler.handle("core", "INFO", "seed")

    race = _Race(
        start=threading.Barrier(6),
        handler=handler,
        handle_error=handle_error,
        failures=[],
    )
    threads = _start_threads(race)
    _join_all(threads, budget=10)

    try:
        _assert_joined(threads, race.failures)
        _assert_live_diagnostics(handler)
        _assert_post_handoff(handler, handle_error)
    finally:
        # Idempotent, and it releases any producer still waiting for capacity
        # if a closer failed before it could shut the worker down.
        handler.close()
