"""Run re-entrant finalizer scenarios in a bounded child interpreter.

The scenarios under test need a fresh interpreter: a deadlocked finalizer would
hang the pytest process itself, and a process-wide logging manager reset in the
middle of collection teardown would confuse later tests. Running each script
with ``python -c`` and a hard timeout makes a hang a test failure instead.

A marker on standard output is the completion signal. Scripts print it only
after the re-entrant log call inside ``__del__`` returns, so a script that
merely avoids crashing without finishing the re-entry still fails. For the
same reason each finalizer records completion after its log call, never
before: Python swallows an exception raised by ``__del__``, and a flag set
first would let the marker stand for a re-entry that never happened.
"""

from __future__ import annotations

import subprocess  # ruff: ignore[suspicious-subprocess-import] fixed argv, no shell
import sys

#: How long a scenario may take before it is treated as deadlocked.
CHILD_TIMEOUT_SECONDS = 10.0


def run_reentrant_finalizer_script(
    script: str,
    marker: str,
    *,
    timeout: float = CHILD_TIMEOUT_SECONDS,
) -> None:
    """Run *script* in a child interpreter and require a *marker* output line.

    Parameters
    ----------
    script
        Python source to execute with the current interpreter's ``-c`` flag.
    marker
        Exact line the script prints after its re-entrant call returns.
    timeout
        Seconds the child may run before it is killed and the test fails.

    Raises
    ------
    AssertionError
        If the child times out, exits non-zero, or does not print ``marker``.
        The captured standard output and error are included so a deadlock is
        distinguishable from an exception in the scenario.

    Examples
    --------
    >>> run_reentrant_finalizer_script(
    ...     "print('done')",
    ...     "done",
    ... )
    """
    try:
        result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed argv, no shell
            [sys.executable, "-c", script],
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        message = (
            f"child interpreter did not finish within {timeout}s; "
            f"the finalizer most likely deadlocked\n"
            f"stdout:\n{_as_text(exc.stdout)}\nstderr:\n{_as_text(exc.stderr)}"
        )
        raise AssertionError(message) from exc

    assert result.returncode == 0, (  # ruff: ignore[assert] test outcome check
        f"child interpreter exited with {result.returncode}\n"
        f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
    )
    # The marker must be a whole line: a scenario that prints it embedded in
    # other text has not reached the point the marker stands for.
    assert marker in result.stdout.splitlines(), (  # ruff: ignore[assert] test outcome check
        f"child interpreter never printed the completion marker {marker!r}\n"
        f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
    )


def _as_text(output: str | bytes | None) -> str:
    """Decode captured child output for an assertion message."""
    if output is None:
        return "<none>"
    if isinstance(output, bytes):
        return output.decode(errors="replace")
    return output


CLEAR_HANDLERS_MARKER = "clear-handlers-finalizer-complete"
CLEAR_HANDLERS_SCRIPT = """
import gc

from femtologging import get_logger

NAME = "reentrant.clear.handlers"
logger = get_logger(NAME)
phase = "setup"
dropped = []


class FinalizingHandler:
    def handle(self, logger_name, level, message):
        pass

    def __del__(self):
        assert phase != "setup", "the handler finalised before clear_handlers"
        get_logger(NAME).info("re-entered from a handler finalizer")
        dropped.append(True)


handler = FinalizingHandler()
logger.add_handler(handler)
del handler
gc.collect()
assert not dropped, "the logger did not keep the handler alive"
phase = "clear"
logger.clear_handlers()
assert dropped == [True], "clear_handlers did not finalise the handler"
print("clear-handlers-finalizer-complete")
"""

REMOVE_HANDLER_MARKER = "remove-handler-finalizer-complete"
REMOVE_HANDLER_SCRIPT = """
import gc

from femtologging import get_logger


class Collector:
    def __init__(self):
        self.records = []

    def handle(self, logger_name, level, message):
        self.records.append((logger_name, level, message))


NAME = "reentrant.remove.handler"
logger = get_logger(NAME)
collector = Collector()
logger.add_handler(collector)


class FinalizingHandler:
    def handle(self, logger_name, level, message):
        pass

    def __del__(self):
        get_logger(NAME).info("re-entered from a removed handler")
        print("remove-handler-finalizer-complete")


handler = FinalizingHandler()
logger.add_handler(handler)
assert logger.remove_handler(handler), "the finalizing handler was not attached"
del handler
gc.collect()
assert logger.flush_handlers(), "the finalizer record was not flushed"
assert [record[2] for record in collector.records] == [
    "re-entered from a removed handler"
], collector.records
"""

BASIC_CONFIG_MARKER = "basic-config-finalizer-complete"
BASIC_CONFIG_SCRIPT = """
import gc

from femtologging import basicConfig, get_logger

root = get_logger("root")
phase = "setup"
dropped = []


class FinalizingHandler:
    def handle(self, logger_name, level, message):
        pass

    def __del__(self):
        assert phase != "setup", "the handler finalised before basicConfig"
        get_logger("root").info("re-entered from a forced handler removal")
        dropped.append(True)


handler = FinalizingHandler()
root.add_handler(handler)
del handler
gc.collect()
assert not dropped, "the root logger did not keep the handler alive"
phase = "force"
basicConfig(force=True, handlers=[])
assert dropped == [True], "basicConfig(force=True) did not finalise the handler"
print("basic-config-finalizer-complete")
"""

CLEAR_FILTERS_MARKER = "clear-filters-finalizer-complete"
CLEAR_FILTERS_SCRIPT = """
import gc
import logging

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    PythonCallbackFilterBuilder,
    get_logger,
)

NAME = "reentrant.clear.filters"
phase = "setup"
dropped = []


class FinalizingCallback:
    def __call__(self, record: logging.LogRecord) -> bool:
        return True

    def __del__(self):
        assert phase != "setup", "the callback finalised before clear_filters"
        get_logger(NAME).info("re-entered from a filter finalizer")
        dropped.append(True)


callback = FinalizingCallback()
(
    ConfigBuilder()
    .with_filter("cb", PythonCallbackFilterBuilder(callback))
    .with_logger(NAME, LoggerConfigBuilder().with_filters(["cb"]))
    .with_root_logger(LoggerConfigBuilder())
    .build_and_init()
)
# A root-only configuration releases the registry's reference without touching
# the live logger, leaving its filter collection as the last owner.
(
    ConfigBuilder()
    .with_root_logger(LoggerConfigBuilder())
    .with_disable_existing_loggers(False)
    .build_and_init()
)
del callback
gc.collect()
# The root-only reconfiguration must not have released the logger's filter;
# if it did, this assertion fails instead of the marker passing by accident.
assert not dropped, "the reconfiguration already finalised the callback"
phase = "clear"
get_logger(NAME).clear_filters()
assert dropped == [True], "clear_filters did not finalise the callback"
print("clear-filters-finalizer-complete")
"""

REPLACE_CONFIGURATION_MARKER = "replace-configuration-finalizer-complete"
REPLACE_CONFIGURATION_SCRIPT = """
import gc
import logging

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    PythonCallbackFilterBuilder,
    get_logger,
)

NAME = "reentrant.replace.configuration"
phase = "setup"
dropped = []


class FinalizingCallback:
    def __call__(self, record: logging.LogRecord) -> bool:
        return True

    def __del__(self):
        assert phase != "setup", "the callback finalised before the replacement"
        get_logger(NAME).info("re-entered during configuration replacement")
        dropped.append(True)


callback = FinalizingCallback()
(
    ConfigBuilder()
    .with_filter("cb", PythonCallbackFilterBuilder(callback))
    .with_logger(NAME, LoggerConfigBuilder().with_filters(["cb"]))
    .with_root_logger(LoggerConfigBuilder())
    .build_and_init()
)
del callback
gc.collect()
assert not dropped, "the configuration did not keep the callback alive"
phase = "replace"
# Reconfiguring the logger without the filter displaces it from both the
# registry and the live logger in one transaction commit.
(
    ConfigBuilder()
    .with_logger(NAME, LoggerConfigBuilder())
    .with_root_logger(LoggerConfigBuilder())
    .build_and_init()
)
assert dropped == [True], "the replacement did not finalise the callback"
print("replace-configuration-finalizer-complete")
"""

RESET_MANAGER_MARKER = "reset-manager-finalizer-complete"
RESET_MANAGER_SCRIPT = """
import gc
import logging

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    PythonCallbackFilterBuilder,
    get_logger,
    reset_manager,
)

NAME = "reentrant.reset.manager"
phase = "setup"
dropped = []


class FinalizingCallback:
    def __call__(self, record: logging.LogRecord) -> bool:
        return True

    def __del__(self):
        assert phase != "setup", "the callback finalised before reset_manager"
        get_logger(NAME).info("re-entered from reset_manager")
        dropped.append(True)


callback = FinalizingCallback()
(
    ConfigBuilder()
    .with_filter("cb", PythonCallbackFilterBuilder(callback))
    .with_logger(NAME, LoggerConfigBuilder().with_filters(["cb"]))
    .with_root_logger(LoggerConfigBuilder())
    .build_and_init()
)
del callback
gc.collect()
# Release the live logger's reference so the runtime registry is the last
# owner; reset_manager then destroys it after dropping MANAGER.
get_logger(NAME).clear_filters()
assert not dropped, "clearing the live logger finalised the callback too early"
phase = "reset"
reset_manager()
assert dropped == [True], "reset_manager did not finalise the callback"
print("reset-manager-finalizer-complete")
"""
