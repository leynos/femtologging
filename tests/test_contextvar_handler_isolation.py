"""Property coverage for producer-context isolation across Python handlers."""

from __future__ import annotations

import contextvars
import threading

from hypothesis import given, settings
from hypothesis import strategies as st

from femtologging import FemtoLogger

_REQUEST_ID = contextvars.ContextVar[str | None]("request_id", default=None)
_REQUEST_IDS = st.lists(
    st.text(min_size=1, max_size=24),
    min_size=1,
    max_size=4,
    unique=True,
)
_HANDLER_ORDERS = st.permutations(("structured", "legacy"))


def _context_handlers(
    observed: list[tuple[str, str, str | None]],
    observed_lock: threading.Lock,
    observed_event: threading.Event,
    expected_count: int,
) -> dict[str, object]:
    """Build structured and legacy handlers that capture producer context."""

    def observe(handler_kind: str, message: str) -> None:
        """Store the value visible to one callback and signal completion."""
        with observed_lock:
            observed.append((handler_kind, message, _REQUEST_ID.get()))
            if len(observed) == expected_count:
                observed_event.set()

    class StructuredHandler:
        """Observe records through the structured Python handler protocol."""

        @staticmethod
        def handle(_logger: str, _level: str, _message: str) -> None:
            """Satisfy the legacy handler protocol required at registration."""

        @staticmethod
        def handle_record(record: dict[str, str]) -> None:
            """Observe the producer context through the structured path."""
            observe("structured", record["message"])

    class LegacyHandler:
        """Observe records through the legacy Python handler protocol."""

        @staticmethod
        def handle(_logger: str, _level: str, message: str) -> None:
            """Observe the producer context through the legacy path."""
            observe("legacy", message)

    return {"structured": StructuredHandler(), "legacy": LegacyHandler()}


def _configured_logger(
    handler_order: tuple[str, str],
    handlers: dict[str, object],
) -> FemtoLogger:
    """Return a logger with handlers attached in the generated order."""
    logger = FemtoLogger("contextvars.property")
    for handler_kind in handler_order:
        logger.add_handler(handlers[handler_kind])
    return logger


def _emit_from_producers(logger: FemtoLogger, request_ids: list[str]) -> None:
    """Emit one record per concurrently started producer thread."""
    start = threading.Barrier(len(request_ids) + 1)

    def emit(index: int, request_id: str) -> None:
        """Emit one record from a thread carrying ``request_id``."""
        token = _REQUEST_ID.set(request_id)
        try:
            start.wait()
            logger.info(f"record-{index}")
        finally:
            _REQUEST_ID.reset(token)

    threads = [
        threading.Thread(target=emit, args=(index, request_id))
        for index, request_id in enumerate(request_ids)
    ]
    for thread in threads:
        thread.start()
    start.wait()
    for thread in threads:
        thread.join()


def _expected_observations(
    request_ids: list[str],
    handler_order: tuple[str, str],
) -> set[tuple[str, str, str]]:
    """Return every handler-record-context tuple the callbacks must observe."""
    return {
        (handler_kind, f"record-{index}", request_id)
        for index, request_id in enumerate(request_ids)
        for handler_kind in handler_order
    }


@settings(max_examples=25, deadline=None)
@given(request_ids=_REQUEST_IDS, handler_order=_HANDLER_ORDERS)
def test_contextvar_snapshot_matches_each_producer(
    request_ids: list[str],
    handler_order: tuple[str, str],
) -> None:
    """Each handler should observe every record's originating context value."""
    observed: list[tuple[str, str, str | None]] = []
    observed_lock = threading.Lock()
    observed_event = threading.Event()
    expected_count = len(request_ids) * len(handler_order)
    handlers = _context_handlers(
        observed,
        observed_lock,
        observed_event,
        expected_count,
    )
    logger = _configured_logger(handler_order, handlers)
    _emit_from_producers(logger, request_ids)

    assert observed_event.wait(timeout=1.0), (
        f"expected {expected_count} callbacks, got {observed!r}"
    )
    assert logger.flush_handlers(), "logger worker did not flush"

    expected = _expected_observations(request_ids, handler_order)
    assert set(observed) == expected, f"context values crossed records: {observed!r}"
