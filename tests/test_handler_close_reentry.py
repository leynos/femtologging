"""Regression tests for Python callbacks during worker shutdown."""

from __future__ import annotations

import dataclasses
import threading
import time
import typing as typ

import pytest

from femtologging import (
    FileHandlerBuilder,
    RotatingFileHandlerBuilder,
    StreamHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
    rust,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

# Reports how many native stream-handler calls are blocked in a wait. It is
# present only when the extension was built with the Rust `test-util` feature,
# which `make develop` enables for the test suite.
_STREAM_WAITER_COUNT: cabc.Callable[[], int] | None = getattr(
    rust, "_stream_waiter_count", None
)

pytestmark = pytest.mark.skipif(
    _STREAM_WAITER_COUNT is None,
    reason="requires Rust extension built with the 'test-util' feature",
)

# A close that never reaches its wait is reported as a test failure instead of
# hanging the worker, so the deadline bounds a broken run rather than a slow
# one: the wait is normally visible within a few milliseconds.
_CLOSE_ENTRY_TIMEOUT_SECONDS = 5.0
_CLOSE_ENTRY_POLL_SECONDS = 0.005


class _ClosableHandler(typ.Protocol):
    """Python methods shared by the worker-backed handler types."""

    def handle(self, logger: str, level: str, message: str) -> None:
        """Accept one record for the worker to format and write."""

    def flush(self) -> bool:
        """Flush pending records and report whether the flush succeeded."""

    def close(self) -> None:
        """Close the handler and drain its worker."""


class _DiagnosticHandler(typ.Protocol):
    """File-family surface that reports whether a wait is in flight."""

    def wait_diagnostics(self) -> cabc.Mapping[str, int]:
        """Report the handler's bounded wait counters."""


def _close_is_waiting(handler: _ClosableHandler, handler_kind: str) -> bool:
    """Report whether ``close()`` has taken the shutdown state and is waiting.

    Taking the shutdown state is the same transition that makes a re-entrant
    ``handle`` observe the closed state. The file family's counter is also
    raised while ``handle()`` and ``flush()`` wait, so the formatter waits for
    the initial ``handle()`` to return before it polls; the stream counter
    counts only flush and close.

    Returns
    -------
    bool
        ``True`` once the handler's close wait is in flight.
    """
    if handler_kind == "stream":
        return _STREAM_WAITER_COUNT is not None and _STREAM_WAITER_COUNT() > 0
    diagnostics = typ.cast("_DiagnosticHandler", handler).wait_diagnostics()
    return diagnostics["waits_in_flight"] > 0


def _await_close(handler: _ClosableHandler, handler_kind: str) -> None:
    """Block the worker until ``close()`` has taken the shutdown state.

    Without this wait the worker can format the queued record before
    ``close()`` runs, and the re-entrant ``handle`` is then legitimately
    accepted: the test observes an ordinary enqueue rather than the closed
    state it exists to check. The caller must have seen the initial ``handle()``
    return first, or that call's own wait would satisfy the poll.
    """
    deadline = time.monotonic() + _CLOSE_ENTRY_TIMEOUT_SECONDS
    while not _close_is_waiting(handler, handler_kind):
        if time.monotonic() >= deadline:
            # Fall through so the assertions report the outcome instead of
            # the worker holding the suite until its own timeout.
            return
        time.sleep(_CLOSE_ENTRY_POLL_SECONDS)


@dataclasses.dataclass(slots=True)
class _ReentryProbe:
    """Collect what the worker's formatter observes when it re-enters a handler.

    The formatter runs on the handler's worker thread, so its outcomes are
    recorded here instead of being returned to the test, and the handler is
    published after it exists because the formatter needs to call it.
    """

    handler_kind: str
    handle_returned: threading.Event = dataclasses.field(
        default_factory=threading.Event
    )
    handler_ref: list[_ClosableHandler] = dataclasses.field(default_factory=list)
    handle_outcomes: list[str] = dataclasses.field(default_factory=list)
    flush_outcomes: list[bool | str] = dataclasses.field(default_factory=list)

    def formatter(self) -> cabc.Callable[[cabc.Mapping[str, object]], str]:
        """Build a formatter that re-enters the handler once close is waiting."""

        def format_record(record: cabc.Mapping[str, object]) -> str:
            """Re-enter the handler from the formatter and record each outcome."""
            self._reenter_while_closing()
            return str(record.get("message", ""))

        return format_record

    def _reenter_while_closing(self) -> None:
        """Call ``handle`` and ``flush`` once the handler is closing."""
        # ``handle()`` raises the same in-flight gauge ``close()`` raises, and
        # its guard is held across the detached enqueue -- which is exactly the
        # moment this formatter starts. Waiting for that call to return keeps
        # the gate keyed to ``close()`` rather than to the record's own
        # delivery.
        if not self.handle_returned.wait(timeout=_CLOSE_ENTRY_TIMEOUT_SECONDS):
            self.handle_outcomes.append("handle() never returned")
            return
        if not self.handler_ref:
            return
        handler = self.handler_ref[0]
        _await_close(handler, self.handler_kind)
        try:
            handler.handle("callback", "INFO", "during-close")
        except (RuntimeError, ValueError) as error:
            self.handle_outcomes.append(str(error))
        else:
            self.handle_outcomes.append("accepted")

        try:
            self.flush_outcomes.append(handler.flush())
        except RuntimeError as error:
            self.flush_outcomes.append(str(error))


def _build_handler(
    handler_kind: str,
    log_path: Path,
    formatter: cabc.Callable[[cabc.Mapping[str, object]], str],
) -> _ClosableHandler:
    """Build a worker-backed handler with the supplied formatter."""
    match handler_kind:
        case "stream":
            built_handler = (
                StreamHandlerBuilder.stderr().with_formatter(formatter).build()
            )
        case "file":
            built_handler = (
                FileHandlerBuilder(str(log_path)).with_formatter(formatter).build()
            )
        case "rotating":
            built_handler = (
                RotatingFileHandlerBuilder(str(log_path))
                .with_formatter(formatter)
                .build()
            )
        case _:
            built_handler = (
                TimedRotatingFileHandlerBuilder(str(log_path))
                .with_formatter(formatter)
                .build()
            )
    return typ.cast("_ClosableHandler", built_handler)


@pytest.mark.parametrize(
    "handler_kind",
    ["stream", "file", "rotating", "timed-rotating"],
    ids=["stream", "file", "rotating", "timed-rotating"],
)
def test_close_allows_worker_formatter_to_reach_closed_state(
    tmp_path: Path,
    handler_kind: str,
) -> None:
    """Worker callbacks can reach closed-state handling while close waits."""
    probe = _ReentryProbe(handler_kind=handler_kind)
    log_path = tmp_path / f"{handler_kind}.log"
    handler = _build_handler(handler_kind, log_path, probe.formatter())
    probe.handler_ref.append(handler)
    try:
        handler.handle("initial", "INFO", "queued")
    finally:
        # Released only once the enqueue has returned, so the formatter cannot
        # mistake that call's own wait for the close it is meant to observe.
        probe.handle_returned.set()
    try:
        handler.close()
    finally:
        handler.close()

    assert len(probe.handle_outcomes) == 1, (
        "worker formatter should re-enter handle once"
    )
    assert "handler is closed" in probe.handle_outcomes[0], probe.handle_outcomes[0]
    assert probe.flush_outcomes == [False], probe.flush_outcomes
