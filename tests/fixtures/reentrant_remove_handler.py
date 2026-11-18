"""Remove one handler and let its finalizer log to the surviving collector.

The caller keeps a reference past removal, so this scenario proves the
re-entry completes through dispatch rather than that the collection destroyed
its last owner. The record the finalizer emits is checked, which also proves
the log call itself finished.

Run by ``tests.reentrant_finalizer_support`` in a child interpreter.
"""

from __future__ import annotations

import gc

from femtologging import get_logger

NAME = "reentrant.remove.handler"


class ScenarioState:
    """Track whether the finalizer has run."""

    def __init__(self) -> None:
        """Start with no finalizer recorded."""
        self.dropped: list[bool] = []


STATE = ScenarioState()


class Collector:
    """Keep every record a logger dispatches to it."""

    def __init__(self) -> None:
        """Start with an empty record list."""
        self.records: list[tuple[str, str, str]] = []

    def handle(self, logger_name: str, level: str, message: str) -> None:
        """Store one record for the scenario's final assertion."""
        self.records.append((logger_name, level, message))


class FinalizingHandler:
    """Handler whose finalizer logs to the logger that removed it."""

    def handle(self, logger_name: str, level: str, message: str) -> None:
        """Accept the record; the collector observes the output instead."""

    def __del__(self) -> None:
        """Re-enter the logger from finalization."""
        get_logger(NAME).info("re-entered from a removed handler")
        STATE.dropped.append(True)


def main() -> None:
    """Run the scenario and print the completion marker."""
    logger = get_logger(NAME)
    collector = Collector()
    logger.add_handler(collector)
    handler = FinalizingHandler()
    logger.add_handler(handler)
    assert logger.remove_handler(handler), "the finalizing handler was not attached"
    del handler
    gc.collect()
    assert STATE.dropped == [True], "removing the handler did not finalise it"
    assert logger.flush_handlers(), "the finalizer record was not flushed"
    assert [record[2] for record in collector.records] == [
        "re-entered from a removed handler"
    ], collector.records
    print("remove-handler-finalizer-complete")


if __name__ == "__main__":
    main()
