"""Handler attribute lookup must use the producer's ``contextvars`` context."""

from __future__ import annotations

import contextvars
import threading
import typing as typ

from femtologging import FemtoLogger

if typ.TYPE_CHECKING:
    import collections.abc as cabc


def test_handler_attribute_lookup_runs_in_producer_context() -> None:
    """Resolve structured and legacy handler descriptors in the captured context."""
    request_id = contextvars.ContextVar[str | None]("request_id", default=None)
    lookups: dict[str, list[str | None]] = {"structured": [], "legacy": []}
    callbacks: list[tuple[str, str | None]] = []
    callbacks_finished = threading.Event()

    def observe_callback(handler_kind: str) -> None:
        callbacks.append((handler_kind, request_id.get()))
        if len(callbacks) == 2:
            callbacks_finished.set()

    class StructuredHandler:
        """Expose a descriptor-backed structured callback."""

        @staticmethod
        def handle(_logger: str, _level: str, _message: str) -> None:
            """Satisfy the handler registration protocol."""

        @property
        def handle_record(
            self,
        ) -> cabc.Callable[[dict[str, object]], None]:
            lookups["structured"].append(request_id.get())

            def invoke(_record: dict[str, object]) -> None:
                observe_callback("structured")

            return invoke

    class LegacyHandler:
        """Expose a descriptor-backed legacy callback."""

        @property
        def handle(self) -> cabc.Callable[[str, str, str], None]:
            lookups["legacy"].append(request_id.get())

            def invoke(_logger: str, _level: str, _message: str) -> None:
                observe_callback("legacy")

            return invoke

    logger = FemtoLogger("contextvars.attribute_lookup")
    registration_token = request_id.set("registration")
    try:
        logger.add_handler(StructuredHandler())
        logger.add_handler(LegacyHandler())
    finally:
        request_id.reset(registration_token)

    producer_token = request_id.set("request-42")
    try:
        logger.info("test")
    finally:
        request_id.reset(producer_token)

    assert callbacks_finished.wait(timeout=1.0), "both handlers should run"
    assert logger.flush_handlers(), "logger worker should flush"
    assert lookups == {
        "structured": ["registration", "request-42"],
        "legacy": ["registration", "request-42"],
    }, f"handler descriptors did not use the producer context: {lookups!r}"
    assert callbacks == [
        ("structured", "request-42"),
        ("legacy", "request-42"),
    ], f"handler callbacks did not use the producer context: {callbacks!r}"
