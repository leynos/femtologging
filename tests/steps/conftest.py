"""Shared BDD steps reused across feature modules."""

from __future__ import annotations

import typing as typ

from pytest_bdd import given, parsers, then

from femtologging import get_logger, reset_manager

if typ.TYPE_CHECKING:
    from syrupy.assertion import SnapshotAssertion


@given("the logging system is reset")
def reset_logging() -> None:
    """Reset global logging state for scenario isolation."""
    reset_manager()


@then(parsers.parse('logging "{msg}" at "{level}" from root matches snapshot'))
def log_matches_snapshot(msg: str, level: str, snapshot: SnapshotAssertion) -> None:
    """Assert root logger output matches snapshot, handling DEBUG specially."""
    logger = get_logger("root")
    formatted = logger.log(level, msg)
    if level.upper() == "DEBUG":
        assert formatted is None
    else:
        assert formatted is not None
        assert formatted == snapshot
