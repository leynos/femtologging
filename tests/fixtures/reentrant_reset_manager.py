"""Reset the manager while the runtime registry owns a finalizing callback.

The live logger's reference is cleared first, so the runtime registry is the
callback's last owner. ``reset_manager`` takes that state out of ``MANAGER``
under the lock and destroys it after releasing the lock, which is what lets the
finalizer's ``get_logger`` call complete.

Checks go through ``scenario_support.require`` rather than ``assert``, which
``python -O`` would remove.

Run by ``tests.reentrant_finalizer_support`` in a child interpreter.
"""

from __future__ import annotations

import gc
import typing as typ

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    PythonCallbackFilterBuilder,
    get_logger,
    reset_manager,
)
from tests.fixtures.scenario_support import require

if typ.TYPE_CHECKING:
    import logging

NAME = "reentrant.reset.manager"


class ScenarioState:
    """Track the phase reached and whether the finalizer has run."""

    def __init__(self) -> None:
        """Start in setup with no finalizer recorded."""
        self.phase = "setup"
        self.dropped: list[bool] = []


STATE = ScenarioState()


class FinalizingCallback:
    """Filter callback whose finalizer logs to the configured logger."""

    def __call__(self, record: logging.LogRecord) -> bool:
        """Accept every record."""
        return True

    def __del__(self) -> None:
        """Re-enter the logger from finalization."""
        require(STATE.phase != "setup", "the callback finalised before reset_manager")
        get_logger(NAME).info("re-entered from reset_manager")
        STATE.dropped.append(True)


def configure(callback: FinalizingCallback) -> None:
    """Install *callback* as the filter on ``NAME`` beside a plain root logger."""
    (
        ConfigBuilder()
        .with_filter("cb", PythonCallbackFilterBuilder(callback))
        .with_logger(NAME, LoggerConfigBuilder().with_filters(["cb"]))
        .with_root_logger(LoggerConfigBuilder())
        .build_and_init()
    )


def main() -> None:
    """Run the scenario and print the completion marker."""
    callback = FinalizingCallback()
    configure(callback)
    del callback
    gc.collect()
    # Release the live logger's reference so the runtime registry is the last
    # owner; reset_manager then destroys it after dropping MANAGER.
    get_logger(NAME).clear_filters()
    require(not STATE.dropped, "clearing the live logger finalised the callback early")
    STATE.phase = "reset"
    reset_manager()
    require(STATE.dropped == [True], "reset_manager did not finalise the callback")
    print("reset-manager-finalizer-complete")


if __name__ == "__main__":
    main()
