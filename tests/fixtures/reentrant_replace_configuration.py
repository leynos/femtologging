"""Replace a configuration that owned a callback and finalize it.

Reconfiguring the logger without the filter displaces the callback from both the
registry and the live logger in one transaction commit, so the finalizer runs
as part of the commit rather than at process exit.

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

NAME = "reentrant.replace.configuration"


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
        assert STATE.phase != "setup", "the callback finalised before the replacement"
        get_logger(NAME).info("re-entered during configuration replacement")
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
    assert not STATE.dropped, "the configuration did not keep the callback alive"
    STATE.phase = "replace"
    (
        ConfigBuilder()
        .with_logger(NAME, LoggerConfigBuilder())
        .with_root_logger(LoggerConfigBuilder())
        .build_and_init()
    )
    assert STATE.dropped == [True], "the replacement did not finalise the callback"
    print("replace-configuration-finalizer-complete")


if __name__ == "__main__":
    main()
