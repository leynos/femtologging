"""Clear a logger's filters while a callback finalizer re-enters it.

A root-only reconfiguration releases the registry's reference without touching
the live logger, so its filter collection becomes the callback's last owner and
``clear_filters`` runs the finalizer.

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
)

if typ.TYPE_CHECKING:
    import logging

NAME = "reentrant.clear.filters"


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
        assert STATE.phase != "setup", "the callback finalised before clear_filters"
        get_logger(NAME).info("re-entered from a filter finalizer")
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
    # The builder takes the flag positionally; name it so the reader can see
    # that this reconfiguration leaves existing loggers enabled.
    disable_existing = False
    (
        ConfigBuilder()
        .with_root_logger(LoggerConfigBuilder())
        .with_disable_existing_loggers(disable_existing)
        .build_and_init()
    )
    del callback
    gc.collect()
    assert not STATE.dropped, "the reconfiguration already finalised the callback"
    STATE.phase = "clear"
    get_logger(NAME).clear_filters()
    assert STATE.dropped == [True], "clear_filters did not finalise the callback"
    print("clear-filters-finalizer-complete")


if __name__ == "__main__":
    main()
