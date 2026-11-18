"""Force a reconfiguration that finalizes a root handler re-entrantly.

``basicConfig(force=True, handlers=[])`` displaces the root logger's handlers
in one commit. The displaced handler is the last owner of a Python object whose
finalizer logs to the root logger, so the finalizer must run after the
reconfiguration has released its locks.

Checks go through ``scenario_support.require`` rather than ``assert``, which
``python -O`` would remove.

Run by ``tests.reentrant_finalizer_support`` in a child interpreter.
"""

from __future__ import annotations

import gc

from femtologging import basicConfig, get_logger
from tests.fixtures.scenario_support import require

NAME = "root"


class ScenarioState:
    """Track the phase reached and whether the finalizer has run."""

    def __init__(self) -> None:
        """Start in setup with no finalizer recorded."""
        self.phase = "setup"
        self.dropped: list[bool] = []


STATE = ScenarioState()


class FinalizingHandler:
    """Root handler whose finalizer logs to the root logger."""

    def handle(self, logger_name: str, level: str, message: str) -> None:
        """Accept the record; this scenario never inspects handler output."""

    def __del__(self) -> None:
        """Re-enter the root logger from finalization."""
        require(STATE.phase != "setup", "the handler finalised before basicConfig")
        get_logger(NAME).info("re-entered from a forced handler removal")
        STATE.dropped.append(True)


def main() -> None:
    """Run the scenario and print the completion marker."""
    root = get_logger(NAME)
    handler = FinalizingHandler()
    root.add_handler(handler)
    del handler
    gc.collect()
    require(not STATE.dropped, "the root logger did not keep the handler alive")
    STATE.phase = "force"
    basicConfig(force=True, handlers=[])
    require(
        STATE.dropped == [True],
        "basicConfig(force=True) did not finalise the handler",
    )
    print("basic-config-finalizer-complete")


if __name__ == "__main__":
    main()
