"""Clear a logger's handlers while one of them finalizes re-entrantly.

The handler is the last owner of a Python object whose finalizer logs to the
same logger. Completion is recorded only after that log call returns, so the
completion marker cannot stand for a re-entry that raised.

Run by ``tests.reentrant_finalizer_support`` in a child interpreter.
"""

from __future__ import annotations

import gc

from femtologging import get_logger

NAME = "reentrant.clear.handlers"


class ScenarioState:
    """Track the phase reached and whether the finalizer has run."""

    def __init__(self) -> None:
        """Start in setup with no finalizer recorded."""
        self.phase = "setup"
        self.dropped: list[bool] = []


STATE = ScenarioState()


class FinalizingHandler:
    """Handler whose finalizer logs to the logger that owned it."""

    def handle(self, logger_name: str, level: str, message: str) -> None:
        """Accept the record; this scenario never inspects handler output."""

    def __del__(self) -> None:
        """Re-enter the logger from finalization."""
        assert STATE.phase != "setup", "the handler finalised before clear_handlers"
        get_logger(NAME).info("re-entered from a handler finalizer")
        STATE.dropped.append(True)


def main() -> None:
    """Run the scenario and print the completion marker."""
    logger = get_logger(NAME)
    handler = FinalizingHandler()
    logger.add_handler(handler)
    del handler
    gc.collect()
    assert not STATE.dropped, "the logger did not keep the handler alive"
    STATE.phase = "clear"
    logger.clear_handlers()
    assert STATE.dropped == [True], "clear_handlers did not finalise the handler"
    print("clear-handlers-finalizer-complete")


if __name__ == "__main__":
    main()
