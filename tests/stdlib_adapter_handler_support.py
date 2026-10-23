"""Shared stdlib handler and filter probes for adapter tests.

These helpers expose handler entry, filter decisions, and emitted records so
adapter tests can assert the wrapped handler contract without worker timing.
"""

from __future__ import annotations

import logging
import typing as typ


class CapturingHandler(logging.Handler):
    """Capture records emitted by a stdlib handler for test inspection.

    Attributes
    ----------
    records : list[logging.LogRecord]
        Records emitted after the handler's filters accept them.

    """

    def __init__(self) -> None:
        """Initialize with an empty records list."""
        super().__init__()
        self.records: list[logging.LogRecord] = []

    @typ.override
    def emit(self, record: logging.LogRecord) -> None:
        """Capture a record after the handler's filters accept it.

        Parameters
        ----------
        record
            Record accepted by the handler's filter path.

        """
        self.records.append(record)


class CountingHandler(logging.Handler):
    """Count handler calls and capture records accepted by filters.

    ``handle()`` counts entry into the filter path. ``emit()`` is reached only
    when that path accepts the record.

    Attributes
    ----------
    handle_calls : int
        Number of calls to ``handle()``.
    records : list[logging.LogRecord]
        Records emitted after the handler's filters accept them.

    """

    def __init__(self) -> None:
        """Initialize with zero handle calls and no captured records."""
        super().__init__()
        self.handle_calls = 0
        self.records: list[logging.LogRecord] = []

    @typ.override
    def handle(self, record: logging.LogRecord) -> bool:
        """Count a handler call, then apply filters and emit if accepted.

        Parameters
        ----------
        record
            Record to pass through the handler's filter path.

        Returns
        -------
        bool
            Whether the filters accepted the record.

        """
        self.handle_calls += 1
        return super().handle(record)

    @typ.override
    def emit(self, record: logging.LogRecord) -> None:
        """Capture a record accepted by the handler's filters.

        Parameters
        ----------
        record
            Record accepted by the handler's filter path.

        """
        self.records.append(record)


class CountingFilter(logging.Filter):
    """Accept records while counting filter consultations.

    Attributes
    ----------
    calls : int
        Number of records passed to ``filter()``.

    """

    def __init__(self) -> None:
        """Initialize with a zero filter-call count."""
        super().__init__()
        self.calls = 0

    @typ.override
    def filter(self, record: logging.LogRecord) -> bool:
        """Count a filter consultation and accept the record.

        Parameters
        ----------
        record
            Record being checked by the handler.

        Returns
        -------
        bool
            Always ``True``.

        """
        self.calls += 1
        return True


class RejectingFilter(logging.Filter):
    """Reject every record and count filter consultations.

    Attributes
    ----------
    calls : int
        Number of records passed to ``filter()``.

    """

    def __init__(self) -> None:
        """Initialize with a zero filter-call count."""
        super().__init__()
        self.calls = 0

    @typ.override
    def filter(self, record: logging.LogRecord) -> bool:
        """Count a filter consultation and reject the record.

        Parameters
        ----------
        record
            Record being checked by the handler.

        Returns
        -------
        bool
            Always ``False``.

        """
        self.calls += 1
        return False


def emitted_messages(handler: CapturingHandler | CountingHandler) -> list[str]:
    """Return the messages of every record *handler* emitted.

    Parameters
    ----------
    handler
        Handler whose emitted records are inspected.

    Returns
    -------
    list[str]
        One message per emitted record, in emission order.

    Examples
    --------
    >>> handler = CapturingHandler()
    >>> emitted_messages(handler)
    []

    """
    return [record.getMessage() for record in handler.records]
