"""BDD steps for scoped context propagated through an ancestor handler."""

from __future__ import annotations

import time
import typing as typ

from pytest_bdd import given, parsers, then, when

from femtologging import get_logger, log_context
from tests.steps.logging_macros_support import RecordCollector


class _ContextRecordPayload(typ.TypedDict):
    """Key-values captured for records emitted inside and outside a context."""

    value: list[dict[str, object]]


@given("a root record collector", target_fixture="root_context_collector")
def given_root_record_collector() -> RecordCollector:
    """Attach a structured collector to the root logger for propagation checks."""
    root = get_logger("root")
    root.clear_handlers()
    assert root.flush_handlers(), "root logger worker did not flush before capture"
    collector = RecordCollector()
    root.add_handler(collector)
    return collector


@when(
    parsers.parse(
        'I emit messages through get_logger "{name}" inside context "{key}"="{value}"'
    ),
    target_fixture="context_record_payload",
)
def emit_get_logger_messages_with_context(
    root_context_collector: RecordCollector,
    name: str,
    key: str,
    value: str,
) -> _ContextRecordPayload:
    """Emit records on either side of scoped context through ``get_logger``."""
    logger = get_logger(name)
    logger.info("outside")
    with log_context(**{key: value}):
        logger.info("inside")
    for _ in range(20):
        if len(root_context_collector.records) == 2:
            break
        time.sleep(0.01)
    assert len(root_context_collector.records) == 2, (
        f"expected two propagated records, got {root_context_collector.records!r}"
    )
    return {
        "value": [
            record["metadata"]["key_values"]
            for record in root_context_collector.records
        ]
    }


@then("the ancestor records preserve empty and scoped key-values")
def ancestor_records_preserve_context(
    context_record_payload: _ContextRecordPayload,
) -> None:
    """Assert that propagation keeps both empty and scoped metadata intact."""
    assert context_record_payload["value"] == [
        {},
        {"correlation_id": "abc123"},
    ], f"unexpected propagated key-values: {context_record_payload['value']!r}"
