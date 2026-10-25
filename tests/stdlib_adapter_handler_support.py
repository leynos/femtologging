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


class CountingFilter(logging.Filter):
    """Count filter consultations and return the configured decision.

    Attributes
    ----------
    calls : int
        Number of records passed to ``filter()``.
    should_accept : bool
        Decision returned by ``filter()`` for every record.

    """

    def __init__(self, *, should_accept: bool = True) -> None:
        """Initialize the call count and filter decision.

        Parameters
        ----------
        should_accept
            Whether the filter accepts each record.
        """
        super().__init__()
        self.calls = 0
        self.should_accept = should_accept

    @typ.override
    def filter(self, record: logging.LogRecord) -> bool:
        """Count a filter consultation and return the configured decision.

        Parameters
        ----------
        record
            Record being checked by the handler.

        Returns
        -------
        bool
            Whether this filter accepts the record.

        """
        self.calls += 1
        return self.should_accept


def emitted_messages(handler: CapturingHandler) -> list[str]:
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
