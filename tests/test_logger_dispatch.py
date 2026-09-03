"""Handler dispatch tests for :class:`FemtoLogger`.

Covers the structured ``handle_record`` payload, the fallback to the
legacy three-argument ``handle`` method, and the fact that the dispatch
path is frozen when a handler is registered.
"""

from __future__ import annotations

import typing as typ

from femtologging import FemtoLogger
from tests.logger_support import (
    CollectingHandler,
    MutableHandler,
    RecordCollectingHandler,
    raise_exception,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from femtologging.adapter import FemtoRecord


def _sole_record(handler: RecordCollectingHandler) -> FemtoRecord:
    """Return the single record captured by *handler*.

    Returns
    -------
    FemtoRecord
        The sole record the handler captured.

    Examples
    --------
    >>> handler = RecordCollectingHandler()
    >>> handler.handle_record({"logger": "core"})
    >>> _sole_record(handler)["logger"]
    'core'

    """
    assert len(handler.records) == 1, (
        f"expected exactly one captured record, got {len(handler.records)}"
    )
    return handler.records[0]


def _assert_frames_present(payload: cabc.Mapping[str, object], label: str) -> None:
    """Assert *payload* carries a non-empty ``frames`` list."""
    assert "frames" in payload, f"{label} is missing a 'frames' key: {payload!r}"
    frames = payload["frames"]
    assert isinstance(frames, list), (
        f"{label} frames must be a list, got {type(frames).__name__}"
    )
    assert frames, f"{label} frames must not be empty"


def test_handle_record_receives_structured_payload() -> None:
    """Handlers with handle_record should receive the full record dict."""
    logger = FemtoLogger("core")
    handler = RecordCollectingHandler()
    logger.add_handler(handler)

    sentinel_msg = "sentinel message"
    try:
        raise_exception(ValueError, sentinel_msg)
    except ValueError:
        logger.log("ERROR", "caught", exc_info=True)

    del logger

    record = _sole_record(handler)
    assert record["logger"] == "core", f"unexpected logger name: {record!r}"
    assert record["level"] == "ERROR", f"unexpected level: {record!r}"
    assert record["message"] == "caught", f"unexpected message: {record!r}"

    assert "exc_info" in record, f"record is missing exc_info: {record!r}"
    exc_info = record["exc_info"]
    assert exc_info["type_name"] == "ValueError", (
        f"unexpected exception type name: {exc_info!r}"
    )
    assert exc_info["message"] == sentinel_msg, (
        f"exc_info message should echo the raised exception: {exc_info!r}"
    )

    # The schema version lets consumers detect payload format changes.
    assert "schema_version" in exc_info, (
        f"exc_info is missing schema_version: {exc_info!r}"
    )
    assert isinstance(exc_info["schema_version"], int), (
        f"schema_version must be an int, got {type(exc_info['schema_version'])}"
    )

    _assert_frames_present(exc_info, "exc_info")
    frame = exc_info["frames"][0]
    for key in ("filename", "lineno", "function"):
        assert key in frame, f"first exc_info frame is missing {key!r}: {frame!r}"


def test_handle_record_fallback_to_handle() -> None:
    """Handlers without handle_record should use the 3-arg handle method."""
    logger = FemtoLogger("core")
    handler = CollectingHandler()
    logger.add_handler(handler)
    logger.log("INFO", "test")
    del logger
    assert handler.records == [("core", "INFO", "test")], (
        f"legacy handle() fallback captured {handler.records!r}"
    )


def test_handle_record_includes_stack_info() -> None:
    """handle_record should include stack_info when present."""
    logger = FemtoLogger("core")
    handler = RecordCollectingHandler()
    logger.add_handler(handler)

    logger.log("INFO", "debug", stack_info=True)

    del logger

    record = _sole_record(handler)
    assert "stack_info" in record, f"record is missing stack_info: {record!r}"
    _assert_frames_present(record["stack_info"], "stack_info")


def test_handle_record_includes_both_exc_and_stack_info() -> None:
    """handle_record should include both exc_info and stack_info when present."""
    logger = FemtoLogger("core")
    handler = RecordCollectingHandler()
    logger.add_handler(handler)

    try:
        raise_exception(ValueError, "test error")
    except ValueError:
        logger.log("ERROR", "caught", exc_info=True, stack_info=True)

    del logger

    record = _sole_record(handler)

    assert "exc_info" in record, f"record is missing exc_info: {record!r}"
    assert record["exc_info"]["type_name"] == "ValueError", (
        f"unexpected exception type name: {record['exc_info']!r}"
    )
    assert record["exc_info"]["message"] == "test error", (
        f"unexpected exception message: {record['exc_info']!r}"
    )
    _assert_frames_present(record["exc_info"], "exc_info")

    assert "stack_info" in record, f"record is missing stack_info: {record!r}"
    _assert_frames_present(record["stack_info"], "stack_info")


def test_handler_gains_handle_record_after_registration() -> None:
    """Adding handle_record after registration should not change dispatch.

    Capability detection happens once at registration time. If a handler
    is registered without handle_record and later gains one, the legacy
    handle() method should still be called because the cached capability
    is frozen at registration.
    """
    logger = FemtoLogger("core")
    handler = MutableHandler()

    # Register without handle_record
    logger.add_handler(handler)

    # Dynamically add handle_record after registration
    def late_handle_record(record: FemtoRecord) -> None:
        handler.handle_record_calls.append(record)

    handler.handle_record = late_handle_record

    logger.log("INFO", "test message")
    del logger

    assert handler.handle_calls == [("core", "INFO", "test message")], (
        f"frozen capability should keep using handle(): {handler.handle_calls!r}"
    )
    assert not handler.handle_record_calls, (
        "handle_record() must not run when it was absent at registration: "
        f"{handler.handle_record_calls!r}"
    )


def test_handler_dispatch_path_frozen_at_registration() -> None:
    """The handle_record *dispatch path* stays frozen, even if the method is replaced.

    ``PyHandler::new`` caches only a boolean -- whether the handler had a
    callable ``handle_record`` at registration time -- not the callable
    itself (see ``rust_extension/src/logger/py_handler.rs``). Once that
    boolean is ``True`` for a handler, the runtime never falls back to the
    legacy ``handle()`` method, no matter what ``handle_record`` is rebound
    to afterwards; each call resolves the *current* ``handle_record``
    attribute via a fresh lookup. Replacing ``handle_record`` after
    registration is explicitly documented as unsupported ("mutating the
    handler object after registration results in undefined behaviour",
    ``py_handler.rs``), so *which* closure ends up running is deliberately
    left unasserted here: pinning it would enshrine an accident of the
    current implementation and would break spuriously if the callable were
    ever cached at registration.

    What is contractual, and what this test proves, is the dispatch
    *mechanism*: exactly one ``handle_record`` dispatch occurs, it carries
    the logged payload, and ``handle()`` is never reached once the path
    froze on ``handle_record``. The closures are tagged so the single
    dispatch is observed rather than assumed, which catches a missed
    dispatch or a double dispatch as well as a fallback to ``handle()``.
    """
    logger = FemtoLogger("core")
    handler = MutableHandler()
    dispatch_log: list[str] = []

    # Add handle_record before registration
    def initial_handle_record(record: FemtoRecord) -> None:
        dispatch_log.append("initial")
        handler.handle_record_calls.append(record)

    handler.handle_record = initial_handle_record

    # Register with handle_record present - this freezes the dispatch path
    logger.add_handler(handler)

    # Replace handle_record with a different implementation after registration
    def replacement_handle_record(record: FemtoRecord) -> None:
        dispatch_log.append("replacement")
        handler.handle_record_calls.append(record)

    handler.handle_record = replacement_handle_record

    logger.log("INFO", "test message")
    del logger

    assert len(handler.handle_record_calls) == 1, (
        f"expected one handle_record call, got {len(handler.handle_record_calls)}"
    )
    assert handler.handle_record_calls[0]["message"] == "test message", (
        f"unexpected captured record: {handler.handle_record_calls[0]!r}"
    )
    assert len(dispatch_log) == 1, (
        "exactly one handle_record closure must run per logged record; "
        f"a missed or duplicated dispatch would show here: {dispatch_log!r}"
    )
    assert not handler.handle_calls, (
        "handle() must not run once dispatch froze on handle_record, even "
        f"though the handle_record callable was replaced: {handler.handle_calls!r}"
    )
