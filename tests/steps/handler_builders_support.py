"""Shared type aliases and helpers for the handler builder BDD steps.

``tests/features/handler_builders.feature`` covers every handler builder, so
its step definitions are grouped by handler family across
``handler_builders_*_steps.py`` and star-imported by
``tests/steps/test_handler_builders_steps.py``, which owns the
``scenarios()`` binding. This module holds what those groups share.
"""

from __future__ import annotations

import datetime as dt
import typing as typ
from pathlib import Path

from femtologging import (
    FileHandlerBuilder,
    RotatingFileHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
)

if typ.TYPE_CHECKING:
    from femtologging import (
        HTTPHandlerBuilder,
        SocketHandlerBuilder,
        StreamHandlerBuilder,
    )

type FileBuilder = (
    FileHandlerBuilder | RotatingFileHandlerBuilder | TimedRotatingFileHandlerBuilder
)

FEATURES = Path(__file__).resolve().parents[1] / "features"
FEATURE_FILE = str(FEATURES / "handler_builders.feature")


def normalize_builder_path(data: dict[str, object]) -> dict[str, object]:
    """Return builder dict with path normalized to basename for snapshots.

    Snapshots must not depend on the ``tmp_path`` the test happened to get.

    Returns
    -------
    dict[str, object]
        The same mapping, with ``path`` reduced to its basename.

    Examples
    --------
    >>> normalize_builder_path({"path": "/tmp/x/test.log"})
    {'path': 'test.log'}

    """
    data["path"] = Path(str(data["path"])).name
    return data


def require_rotating_builder(builder: FileBuilder) -> RotatingFileHandlerBuilder:
    """Validate that a file builder targets rotation-specific operations.

    Returns
    -------
    RotatingFileHandlerBuilder
        The same builder, narrowed to the rotating type.

    Raises
    ------
    TypeError
        If *builder* is not a ``RotatingFileHandlerBuilder``.

    Examples
    --------
    >>> require_rotating_builder(RotatingFileHandlerBuilder("a.log")).as_dict()["path"]
    'a.log'

    """
    if not isinstance(builder, RotatingFileHandlerBuilder):
        msg = (
            "rotating builder step requires RotatingFileHandlerBuilder, "
            f"got {type(builder).__name__}"
        )
        raise TypeError(msg)
    return builder


def require_timed_builder(builder: FileBuilder) -> TimedRotatingFileHandlerBuilder:
    """Validate that a file builder targets timed rotation operations.

    Returns
    -------
    TimedRotatingFileHandlerBuilder
        The same builder, narrowed to the timed rotating type.

    Raises
    ------
    TypeError
        If *builder* is not a ``TimedRotatingFileHandlerBuilder``.

    Examples
    --------
    >>> b = TimedRotatingFileHandlerBuilder("a.log")
    >>> require_timed_builder(b) is b
    True

    """
    if not isinstance(builder, TimedRotatingFileHandlerBuilder):
        msg = (
            "timed builder step requires TimedRotatingFileHandlerBuilder, "
            f"got {type(builder).__name__}"
        )
        raise TypeError(msg)
    return builder


def parse_clock_time(time_value: str) -> dt.time:
    """Parse an ``HH:MM:SS`` step argument into a ``datetime.time``.

    Returns
    -------
    datetime.time
        The parsed wall-clock time.

    Examples
    --------
    >>> parse_clock_time("06:30:00")
    datetime.time(6, 30)

    """
    hours, minutes, seconds = (int(part) for part in time_value.split(":"))
    return dt.time(hours, minutes, seconds)


def build_flush_close(
    builder: HTTPHandlerBuilder | SocketHandlerBuilder | StreamHandlerBuilder,
) -> None:
    """Build a handler, confirm its flush, and close it even after a failure."""
    handler = builder.build()
    try:
        assert handler.flush(), "handler.flush() timed out"
    finally:
        handler.close()
