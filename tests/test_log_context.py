"""Regression coverage for Python task-local structured log context."""

from __future__ import annotations

import asyncio
import time
import typing as typ

from femtologging import FemtoLogger, log_context


class _RecordCollector:
    def __init__(self) -> None:
        self.records: list[dict[str, object]] = []
        # The Rust bridge resolves ``handle_record`` with ``getattr`` on the
        # instance, so binding ``list.append`` directly avoids a method that
        # would do nothing but forward the argument.
        self.handle_record = self.records.append

    def handle(self, logger: str, level: str, message: str) -> None:
        _ = (self.records, logger, level, message)

    def flush(self) -> bool:
        _ = self.records
        return True


async def _run_interleaved_context_logs(logger: FemtoLogger) -> None:
    """Emit records from interleaved scoped and unscoped tasks."""
    context_active = asyncio.Event()
    allow_scoped_log = asyncio.Event()

    async def log_with_context() -> None:
        with log_context(request_id="request-a"):
            context_active.set()
            await allow_scoped_log.wait()
            logger.info("scoped")

    async def log_without_context() -> None:
        await context_active.wait()
        logger.info("unscoped")
        allow_scoped_log.set()

    await asyncio.gather(log_with_context(), log_without_context())


def _wait_for_records(logger: FemtoLogger, records: list[dict[str, object]]) -> None:
    """Flush a logger until its asynchronous handler has received both records."""
    for _ in range(20):
        if len(records) == 2:
            return
        logger.flush_handlers()
        time.sleep(0.01)


def test_log_context_is_task_local_across_await() -> None:
    """A task holding context across an await must not enrich another task's log."""
    logger = FemtoLogger("ctx.async")
    collector = _RecordCollector()
    logger.add_handler(collector)

    asyncio.run(_run_interleaved_context_logs(logger))
    _wait_for_records(logger, collector.records)

    assert len(collector.records) == 2, "expected one record from each task"
    metadata_by_message = {
        typ.cast("str", record["message"]): typ.cast(
            "dict[str, object]", record["metadata"]
        )
        for record in collector.records
    }
    assert metadata_by_message["scoped"]["key_values"] == {"request_id": "request-a"}, (
        "scoped task record must retain only its request context"
    )
    assert metadata_by_message["unscoped"]["key_values"] == {}, (
        "unscoped task record must not inherit request context"
    )
