"""Shared BDD steps reused across feature modules."""

from __future__ import annotations

import re
import typing as typ

from pytest_bdd import given, parsers, then

from femtologging import get_logger, reset_manager

if typ.TYPE_CHECKING:
    from syrupy.assertion import SnapshotAssertion


def normalise_traceback_output(output: str | None, placeholder: str = "<file>") -> str:
    """Normalise traceback output for snapshot comparison.

    Replaces file paths and line numbers with stable placeholders.

    Args:
        output: The traceback output string to normalise, or None.
        placeholder: The placeholder to use for file paths (default: "<file>").

    Returns:
        Normalised output string, or empty string if output is None.

    """
    if output is None:
        return ""

    # Replace file paths with placeholder
    result = re.sub(
        r'File "[^"]+"',
        f'File "{placeholder}"',
        output,
    )
    # Replace line numbers
    result = re.sub(r", line \d+,", ", line <N>,", result)
    # Pytest can render this lambda call with extra keyword arguments depending
    # on Python/pytest versions. Keep only the stable prefix for snapshots.
    return re.sub(
        r"(^\s*lambda: runtest_hook\(item=item, \*\*kwds\),).*$",
        r"\1",
        result,
        flags=re.MULTILINE,
    )


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
