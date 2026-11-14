"""Behavioural tests: ``flush_handlers()`` reaches user-defined handlers.

A custom handler registered from Python is flushed through its own ``flush``
method, on the thread that called ``flush_handlers()``, after the logger's
queued records have been dispatched. These tests pin the reported result for
each shape of ``flush`` a handler can present, and that one handler's failure
does not leave the handlers after it unflushed. A ``flush`` that raises
``SystemExit`` is included: reporting it must not end the interpreter.
"""

from __future__ import annotations

import subprocess  # ruff: ignore[suspicious-subprocess-import] runs this interpreter
import sys

import pytest

from femtologging import get_logger

# A fresh interpreter keeps the logger's dispatch shard idle, so the record is
# admitted rather than refused behind earlier tests' work. The handler records
# how many records it had received when its flush ran, which is what proves the
# flush followed dispatch rather than racing it.
DELIVERY_SCRIPT = """
from femtologging import get_logger


class Buffering:
    def __init__(self):
        self.records = []
        self.flushes = 0
        self.records_at_flush = None

    def handle(self, logger, level, message):
        self.records.append(message)

    def flush(self):
        self.flushes += 1
        self.records_at_flush = len(self.records)
        return True


handler = Buffering()
logger = get_logger("custom.flush.delivery")
logger.add_handler(handler)
logger.info("buffered")

assert logger.flush_handlers(), "flush_handlers reported failure"
assert handler.records == ["buffered"], handler.records
assert handler.flushes >= 1, "the handler's flush never ran"
assert handler.records_at_flush == 1, (
    "flush ran before the record was dispatched to the handler"
)
"""

# ``PyErr::print`` restores the error and calls ``PyErr_PrintEx(0)``, which
# exits the process for a ``SystemExit``. A flush that raises one is reported
# like any other failure, so the process must survive and the handler after it
# must still be flushed. A fresh interpreter is needed because the failure being
# guarded against would end the pytest process.
SYSTEM_EXIT_SCRIPT = """
from femtologging import get_logger


class Exiting:
    def handle(self, logger, level, message):
        pass

    def flush(self):
        raise SystemExit(7)


class Recording:
    def __init__(self):
        self.flushes = 0

    def handle(self, logger, level, message):
        pass

    def flush(self):
        self.flushes += 1
        return True


exiting = Exiting()
recording = Recording()
logger = get_logger("custom.flush.system-exit")
logger.add_handler(exiting)
logger.add_handler(recording)

assert logger.flush_handlers() is False, "a SystemExit flush must fail the call"
assert recording.flushes == 1, "the handler after the SystemExit was not flushed"
print("survived")
"""


class _RecordingHandler:
    """Custom handler that counts flush calls and reports a chosen outcome."""

    def __init__(self, *, result: object = True, raises: bool = False) -> None:
        """Configure the outcome every flush reports."""
        self.flushes = 0
        self.result = result
        self.raises = raises

    def handle(self, logger: str, level: str, message: str) -> None:
        """Accept a record without buffering it."""

    def flush(self) -> object:
        """Record the call and report the configured outcome."""
        self.flushes += 1
        if self.raises:
            raise RuntimeError
        return self.result


class _WithoutFlush:
    """Custom handler that implements only the required ``handle`` method."""

    def handle(self, logger: str, level: str, message: str) -> None:
        """Accept a record without buffering it."""


class _NonCallableFlush:
    """Custom handler whose ``flush`` attribute is not callable."""

    flush = 42

    def handle(self, logger: str, level: str, message: str) -> None:
        """Accept a record without buffering it."""


def test_flush_handlers_delivers_buffered_records() -> None:
    """A buffered custom handler receives the record before its flush runs."""
    result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed script
        [sys.executable, "-c", DELIVERY_SCRIPT],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stderr


def test_flush_handlers_survives_a_flush_raising_system_exit() -> None:
    """A ``flush`` raising ``SystemExit`` fails the call without exiting.

    ``PyErr::print`` would run ``sys.excepthook`` and end the process on
    ``SystemExit``; the flush path prints with ``PyErr::display`` instead. The
    script runs in a fresh interpreter so the failure it guards against would
    be observable rather than fatal to the test run.
    """
    result = subprocess.run(  # ruff: ignore[subprocess-without-shell-equals-true] fixed script
        [sys.executable, "-c", SYSTEM_EXIT_SCRIPT],
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    assert "survived" in result.stdout, result.stdout


def test_flush_handlers_reports_a_false_flush() -> None:
    """A handler whose ``flush`` returns ``False`` fails the whole call."""
    handler = _RecordingHandler(result=False)
    logger = get_logger("custom.flush.false")
    logger.add_handler(handler)

    assert logger.flush_handlers() is False, (
        "a handler reporting a failed flush must fail the whole call"
    )
    assert handler.flushes == 1, "the handler's flush did not run exactly once"


def test_flush_handlers_reports_a_raising_flush() -> None:
    """A handler whose ``flush`` raises fails the whole call."""
    handler = _RecordingHandler(raises=True)
    logger = get_logger("custom.flush.raising")
    logger.add_handler(handler)

    assert logger.flush_handlers() is False, (
        "a handler whose flush raised must fail the whole call"
    )
    assert handler.flushes == 1, "the handler's flush did not run exactly once"


def test_flush_handlers_accepts_a_handler_without_flush() -> None:
    """A handler with no ``flush`` method counts as a successful flush."""
    logger = get_logger("custom.flush.missing")
    logger.add_handler(_WithoutFlush())

    assert logger.flush_handlers() is True, (
        "a handler with no flush method has nothing to flush, so the call succeeds"
    )


def test_flush_handlers_accepts_a_non_callable_flush() -> None:
    """A non-callable ``flush`` attribute counts as a successful flush."""
    logger = get_logger("custom.flush.non-callable")
    logger.add_handler(_NonCallableFlush())

    assert logger.flush_handlers() is True, (
        "a non-callable flush attribute is not a flush, so the call succeeds"
    )


@pytest.mark.parametrize(
    ("result", "logger_name"),
    [(None, "custom.flush.none"), (0, "custom.flush.zero")],
    ids=["none", "zero"],
)
def test_flush_handlers_treats_a_non_boolean_flush_as_success(
    result: object, logger_name: str
) -> None:
    """A ``flush`` returning ``None`` or ``0`` succeeds and runs exactly once."""
    handler = _RecordingHandler(result=result)
    logger = get_logger(logger_name)
    logger.add_handler(handler)

    assert logger.flush_handlers() is True, (
        f"a flush returning {result!r} must count as success"
    )
    assert handler.flushes == 1, "the handler's flush did not run exactly once"


def test_flush_handlers_flushes_every_handler_after_a_failure() -> None:
    """A failed flush does not leave the handlers after it unflushed."""
    failing = _RecordingHandler(result=False)
    recording = _RecordingHandler()
    logger = get_logger("custom.flush.order")
    logger.add_handler(failing)
    logger.add_handler(recording)

    assert logger.flush_handlers() is False, (
        "one failed flush must still fail the whole call"
    )
    assert failing.flushes == 1, "the failing handler was not flushed"
    assert recording.flushes == 1, "a failed flush skipped the handler after it"
