# ruff: file-ignore[assert] Test support helpers assert on their callers' behalf.
"""Shared test doubles and assertion helpers for :class:`FemtoLogger` tests.

The :class:`FemtoLogger` suite is split across several feature-focused
modules (core behaviour, exception/stack capture, and handler dispatch).
This module holds the handler test doubles and assertion helpers those
modules share so that each test module stays focused on one feature.
"""

from __future__ import annotations

import typing as typ

if typ.TYPE_CHECKING:
    import collections.abc as cabc

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


def assert_output_contains(output: str | None, *fragments: str) -> None:
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

    Examples
    --------
    >>> assert_output_contains("core [INFO] hi", "core", "hi")

    """
    assert output is not None, (
        f"expected formatted output mentioning {list(fragments)}, got None"
    )
    missing = [fragment for fragment in fragments if fragment not in output]
    assert not missing, f"expected {missing} in output: {output!r}"


class CollectingHandler:
    """Handler recording ``(logger, level, message)`` triples."""

    def __init__(self) -> None:
        """Initialize an empty record buffer."""
        self.records: list[tuple[str, str, str]] = []

    def handle(self, logger: str, level: str, message: str) -> None:
        """Collect handled records for later assertions."""
        self.records.append((logger, level, message))


class RecordCollectingHandler:
    """Handler that uses ``handle_record`` for structured access."""

    def __init__(self) -> None:
        """Initialize an empty record buffer."""
        self.records: list[FemtoRecord] = []

    @staticmethod
    def handle(_logger: str, _level: str, _message: str) -> None:
        """Fallback handle method (required by FemtoLogger validation)."""
        # Should not be called when handle_record is present

    def handle_record(self, record: FemtoRecord) -> None:
        """Collect a snapshot of each record for later assertions."""
        # Snapshot the payload: it crosses the extension boundary, so copying
        # keeps assertions independent of any later mutation.
        self.records.append(typ.cast("FemtoRecord", dict(record)))


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
