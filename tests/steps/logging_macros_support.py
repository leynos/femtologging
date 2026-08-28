"""Shared payload types and helpers for the logging-macro BDD steps.

The step definitions in ``tests/steps/test_logging_macros_steps.py`` bind
one feature file, so they cannot be split across modules. This module
holds the payload types, the record-collecting test double, the step
regular expressions, and the parsing/waiting helpers those steps use, so
the step module stays focused on step definitions.
"""

from __future__ import annotations

import re
import time
import typing as typ
from contextlib import contextmanager
from types import MappingProxyType

from femtologging import debug, error, info, warn

if typ.TYPE_CHECKING:
    import collections.abc as cabc


class LogResultPayload(typ.TypedDict):
    """Payload dict shuttled between ``@when`` and ``@then`` steps."""

    value: str | None


class MetadataPayload(typ.TypedDict):
    """Structured metadata captured from ``handle_record`` callbacks."""

    value: dict[str, str]


class ErrorPayload(typ.TypedDict):
    """Error payload captured for unhappy-path assertions."""

    value: str | None


class RecordMetadataPayload(typ.TypedDict):
    """Subset of record metadata used in these behavioural assertions."""

    key_values: dict[str, object]


class CapturedRecordPayload(typ.TypedDict):
    """Subset of captured record payloads consumed by helper assertions."""

    metadata: RecordMetadataPayload


class FlushableLogger(typ.Protocol):
    """Structural type for logger objects that expose ``flush_handlers``."""

    def flush_handlers(self) -> bool:
        """Flush pending records and return whether the flush succeeded."""

    def clear_handlers(self) -> None:
        """Remove all handlers before test-scoped capture."""

    def add_handler(self, handler: object) -> None:
        """Attach a handler for the current capture scope."""

    def remove_handler(self, handler: object) -> None:
        """Detach a handler when capture scope exits."""


FUNC_MAP: cabc.Mapping[str, cabc.Callable[..., str | None]] = MappingProxyType({
    "info": info,
    "debug": debug,
    "warn": warn,
    "error": error,
})

CALL_WITH_CONTEXT_PATTERN = (
    r'I call (?P<func>\w+) with message "(?P<message>[^"]+)" and name '
    r'"(?P<name>[^"]+)" inside context (?P<context>.+)'
)
CALL_WITH_NAME_PATTERN = (
    r'^I call (?P<func>\w+) with message "(?P<message>[^"]+)" and name '
    r'"(?P<name>[^"]+)"$'
)
CALL_WITH_MESSAGE_PATTERN = r'^I call (?P<func>\w+) with message "(?P<message>[^"]+)"$'
CALL_WITH_NESTED_CONTEXT_PATTERN = (
    r'I call (?P<func>\w+) with message "(?P<message>[^"]+)" and name '
    r'"(?P<name>[^"]+)" inside nested context (?P<contexts>.+)'
)
EXPECT_KEY_VALUES_PATTERN = (
    r"the latest record metadata key_values contain (?P<pairs>.+)"
)


class RecordCollector:
    """Collect full records passed to ``handle_record`` callbacks."""

    def __init__(self) -> None:
        """Initialize collector state for one scenario."""
        self.records: list[CapturedRecordPayload] = []

    @staticmethod
    def handle(logger: str, level: str, message: str) -> None:
        """Accept classic handler calls; arguments intentionally unused."""
        # Satisfy handler protocol signature.
        del logger, level, message

    def handle_record(self, record: CapturedRecordPayload) -> None:
        """Capture a snapshot of each record payload for metadata assertions."""
        # Snapshot the payload: it crosses the extension boundary, so copying
        # keeps assertions independent of any later mutation.
        self.records.append(typ.cast("CapturedRecordPayload", dict(record)))

    @staticmethod
    def flush() -> bool:
        """Report successful flush to satisfy ``flush_handlers`` checks.

        Returns
        -------
        bool
            Always ``True``; the collector has no buffered state.

        """
        return True


def normalize_source_location(output: str) -> str:
    """Replace file paths and line numbers with stable placeholders.

    Returns
    -------
    str
        *output* with paths rewritten to ``<file>`` and line numbers to
        ``<N>`` so snapshots do not depend on the checkout location.

    Examples
    --------
    >>> normalize_source_location("/a/b.py:12")
    '<file>:<N>'

    """
    # Normalize file paths (e.g., /foo/bar/baz.py or C:\foo\bar.py -> <file>)
    # Optional drive letter, forward/back-slash separators, lookahead for :line
    result = re.sub(r"(?:[A-Za-z]:)?[^\s:]+\.py(?=:\d+)", "<file>", output)
    # Normalize line numbers (e.g., :42 -> :<N>)
    return re.sub(r":\d+", ":<N>", result)


def parse_pairs(text: str) -> dict[str, str]:
    """Parse ``"key"="value"`` pairs joined by ``and``.

    Returns
    -------
    dict[str, str]
        The parsed key-value pairs.

    Examples
    --------
    >>> parse_pairs('"a"="1" and "b"="2"')
    {'a': '1', 'b': '2'}

    """
    pattern = re.compile(r'"([^"]+)"="([^"]*)"')
    pairs = dict(pattern.findall(text))
    assert pairs, f"expected at least one key-value pair in {text!r}"
    return pairs


def split_nested_contexts(text: str) -> tuple[str, str]:
    """Split ``outer then inner`` context expressions.

    Returns
    -------
    tuple[str, str]
        The outer and inner context expressions.

    Examples
    --------
    >>> split_nested_contexts('"a"="1" then "a"="2"')
    ('"a"="1"', '"a"="2"')

    """
    outer, sep, inner = text.partition(" then ")
    assert sep, f"expected nested context separator in {text!r}"
    return outer, inner


@contextmanager
def capture_records(logger: FlushableLogger) -> cabc.Iterator[RecordCollector]:
    """Attach a short-lived collector after draining pending records.

    Yields
    ------
    RecordCollector
        The collector attached for the duration of the block.

    Examples
    --------
    >>> with capture_records(logger) as collector:
    ...     info("hello", name="app")

    """
    logger.clear_handlers()
    flushed = logger.flush_handlers()
    assert flushed, "flush_handlers() failed before attaching context collector"
    collector = RecordCollector()
    logger.add_handler(collector)
    try:
        yield collector
    finally:
        logger.remove_handler(collector)


def wait_for_latest_key_values(
    logger: FlushableLogger,
    collector: RecordCollector,
    *,
    attempts: int = 20,
    interval_s: float = 0.01,
) -> dict[str, object]:
    """Wait for a captured record and return its latest key-values payload.

    Returns
    -------
    dict[str, object]
        The ``key_values`` mapping of the most recently captured record.

    Examples
    --------
    >>> wait_for_latest_key_values(logger, collector)
    {'request_id': '42'}

    """
    for _ in range(attempts):
        if collector.records:
            break
        time.sleep(interval_s)
        flushed = logger.flush_handlers()
        assert flushed, "flush_handlers() failed while waiting for captured records"
    assert collector.records, "expected at least one captured record"
    return collector.records[-1]["metadata"]["key_values"]
