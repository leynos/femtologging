# ruff: file-ignore[assert] Test support helpers assert on their callers' behalf.
"""Shared test doubles and assertion helpers for :class:`FemtoLogger` tests.

The :class:`FemtoLogger` suite is split across several feature-focused
modules (core behaviour, exception/stack capture, and handler dispatch).
This module holds the handler test doubles and assertion helpers those
modules share so that each test module stays focused on one feature.
"""

from __future__ import annotations

import threading
import typing as typ
from functools import partial

from femtologging import _femtologging_rs

cancel_flush_wait_for_test = getattr(
    _femtologging_rs,
    "cancel_flush_wait_for_test",
    None,
)
wait_for_flush_entry_for_test = getattr(
    _femtologging_rs,
    "wait_for_flush_entry_for_test",
    None,
)
FLUSH_TEST_HOOKS_AVAILABLE = callable(cancel_flush_wait_for_test) and callable(
    wait_for_flush_entry_for_test
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from types import TracebackType

    from femtologging import FemtoLogger
    from femtologging.adapter import FemtoRecord


def raise_exception(
    exc_type: type[BaseException] = ValueError, msg: str = ""
) -> typ.NoReturn:
    """Raise an exception of the given type for testing.

    Parameters
    ----------
    exc_type
        The exception class to raise. Defaults to ValueError.
    msg
        The exception message. Defaults to empty string.

    Examples
    --------
    >>> try:
    ...     raise_exception(KeyError, "missing")
    ... except KeyError as exc:
    ...     str(exc)
    "'missing'"

    """
    if msg:
        raise exc_type(msg)
    raise exc_type()


def assert_output_contains(
    output: str | None, *fragments: str, context: str = ""
) -> None:
    """Assert *output* was produced and mentions every fragment.

    Collecting the fragments into a single assertion keeps the failure
    message complete: every missing fragment is reported at once rather
    than only the first.

    Parameters
    ----------
    output
        Value returned by ``FemtoLogger.log``; ``None`` means the record
        was suppressed.
    fragments
        Substrings that must all be present in *output*.
    context
        Optional label describing the calling scenario, woven into the
        failure messages when supplied.

    Examples
    --------
    >>> assert_output_contains("core [INFO] hi", "core", "hi")
    >>> assert_output_contains("core [INFO] hi", "core", context="direct call")

    """
    prefix = f"{context}: " if context else ""
    if context:
        assert output is not None, f"{prefix}the logger emitted nothing"
    else:
        assert output is not None, (
            f"expected formatted output mentioning {list(fragments)}, got None"
        )
    missing = [fragment for fragment in fragments if fragment not in output]
    if context:
        assert not missing, (
            f"{prefix}rendered record omitted {missing!r}; got {output!r}"
        )
    else:
        assert not missing, f"expected {missing} in output: {output!r}"


class CollectingHandler:
    """Handler recording ``(logger, level, message)`` triples."""

    callback_gate: cabc.Callable[[], None] | None

    def __init__(self) -> None:
        """Initialize an empty record buffer."""
        self.records: list[tuple[str, str, str]] = []
        self.callback_gate = None

    def handle(self, logger: str, level: str, message: str) -> None:
        """Collect handled records for later assertions."""
        if self.callback_gate is not None:
            self.callback_gate()
        self.records.append((logger, level, message))


class RecordCollectingHandler:
    """Handler that uses ``handle_record`` for structured access."""

    callback_gate: cabc.Callable[[], None] | None

    def __init__(self) -> None:
        """Initialize an empty record buffer."""
        self.records: list[FemtoRecord] = []
        self.callback_gate = None

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback handle method (required by FemtoLogger validation)."""
        # Should not be called when handle_record is present

    def handle_record(self, record: FemtoRecord) -> None:
        """Collect a snapshot of each record for later assertions."""
        if self.callback_gate is not None:
            self.callback_gate()
        # Snapshot the payload: it crosses the extension boundary, so copying
        # keeps assertions independent of any later mutation.
        self.records.append(typ.cast("FemtoRecord", dict(record)))


class HandlerFlushGate:
    """Hold a collector callback until the logger enters its flush method.

    A helper thread waits for an explicit test-only signal from the token-free
    Rust flush method before releasing the callback.
    """

    _WAIT_TIMEOUT_S = 3.0

    def __init__(
        self,
        handler: RecordCollectingHandler | CollectingHandler,
        logger: FemtoLogger,
    ) -> None:
        """Install a bounded gate around *handler*'s Python callback.

        Parameters
        ----------
        handler : RecordCollectingHandler | CollectingHandler
            Handler whose callback remains blocked until the flush starts.
        logger : FemtoLogger
            Logger whose token-free flush entry coordinates callback release.

        Raises
        ------
        TypeError
            If the Rust extension does not expose its test-util hooks.
        """
        wait_for_flush_entry = wait_for_flush_entry_for_test
        cancel_flush_wait = cancel_flush_wait_for_test
        if not callable(wait_for_flush_entry) or not callable(cancel_flush_wait):
            message = "HandlerFlushGate requires the Rust test-util feature"
            raise TypeError(message)

        self._handler = handler
        self._wait_for_flush_entry = partial(
            wait_for_flush_entry,
            logger,
            3_000,
        )
        self._cancel_flush_wait = partial(cancel_flush_wait, logger)
        self._callback_started = threading.Event()
        self._callback_release = threading.Event()
        handler.callback_gate = self._wait_for_release
        self._release_thread = threading.Thread(
            target=self._release_after_flush_entry,
            daemon=True,
        )
        self._release_thread.start()

    def __enter__(self) -> HandlerFlushGate:
        """Return this active callback gate."""
        return self

    def __exit__(
        self,
        _exc_type: type[BaseException] | None,
        _exc_value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None:
        """Release the callback and stop the helper thread."""
        self.close()

    def close(self) -> None:
        """Release a pending callback and stop the helper thread."""
        self._handler.callback_gate = None
        callback_was_released = self._callback_release.is_set()
        self._callback_release.set()
        self._callback_started.set()
        # A successful gate consumed Entered; avoid queuing a stale Cancel.
        if not callback_was_released:
            self._cancel_flush_wait()
        self._release_thread.join(timeout=1.0)

    def _wait_for_release(self) -> None:
        """Block handler delivery until the flush gate releases it."""
        self._callback_started.set()
        if not self._callback_release.wait(timeout=self._WAIT_TIMEOUT_S):
            message = "flush did not release the handler callback gate"
            raise TimeoutError(message)

    def _release_after_flush_entry(self) -> None:
        """Release the callback only after the logger enters its flush method."""
        callback_started = self._callback_started.wait(timeout=self._WAIT_TIMEOUT_S)
        if callback_started and self._wait_for_flush_entry():
            self._callback_release.set()


def handler_payloads(
    handler: RecordCollectingHandler | CollectingHandler,
) -> list[tuple[str, str, str]]:
    """Return a common payload shape for either Python handler protocol.

    Parameters
    ----------
    handler : RecordCollectingHandler | CollectingHandler
        Handler whose collected records are normalized.

    Returns
    -------
    list[tuple[str, str, str]]
        ``(logger, level, message)`` tuples for each collected record.

    Raises
    ------
    TypeError
        If *handler* does not implement a supported collector protocol.

    Examples
    --------
    >>> handler = CollectingHandler()
    >>> handler.records.append(("core", "INFO", "hello"))
    >>> handler_payloads(handler)
    [('core', 'INFO', 'hello')]

    """
    match handler:
        case RecordCollectingHandler():
            return [
                (record["logger"], record["level"], record["message"])
                for record in handler.records
            ]
        case CollectingHandler():
            return handler.records
        case _:
            message = f"unsupported handler type: {type(handler).__name__}"
            raise TypeError(message)


class MutableHandler:
    """Handler whose capabilities can be mutated after construction."""

    handle_record: cabc.Callable[[FemtoRecord], None]

    def __init__(self) -> None:
        """Initialize an empty record buffer for both dispatch paths."""
        self.handle_calls: list[tuple[str, str, str]] = []
        self.handle_record_calls: list[FemtoRecord] = []

    def handle(self, logger: str, level: str, message: str) -> None:
        """Legacy 3-argument handle method."""
        self.handle_calls.append((logger, level, message))
