"""Unit tests for filter builders and logger filtering behaviour."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import (
    ConfigBuilder,
    LevelFilterBuilder,
    LoggerConfigBuilder,
    NameFilterBuilder,
    get_logger,
)

if typ.TYPE_CHECKING:
    from femtologging import FemtoLogger


def assert_emitted(logger: FemtoLogger, level: str, message: str, context: str) -> None:
    """Assert that ``logger`` emits a record for the given level and message.

    Parameters
    ----------
    logger:
        Logger under test.
    level:
        Level name passed to :meth:`FemtoLogger.log`.
    message:
        Message passed to :meth:`FemtoLogger.log`.
    context:
        Short description of the filtering expectation, used on failure.
    """
    assert logger.log(level, message) is not None, (
        f"{context}: {level} record {message!r} should pass the active filters"
    )


def assert_suppressed(
    logger: FemtoLogger, level: str, message: str, context: str
) -> None:
    """Assert that ``logger`` drops a record for the given level and message.

    Parameters
    ----------
    logger:
        Logger under test.
    level:
        Level name passed to :meth:`FemtoLogger.log`.
    message:
        Message passed to :meth:`FemtoLogger.log`.
    context:
        Short description of the filtering expectation, used on failure.
    """
    assert logger.log(level, message) is None, (
        f"{context}: {level} record {message!r} should be rejected by the "
        "active filters"
    )


def test_logger_with_multiple_filters() -> None:
    """Combined filters should gate log records accordingly."""
    cb = (
        ConfigBuilder()
        .with_filter("lvl", LevelFilterBuilder().with_max_level("INFO"))
        .with_filter("name", NameFilterBuilder().with_prefix("multi"))
        .with_logger(
            "multi",
            LoggerConfigBuilder().with_filters(["lvl", "name"]),
        )
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
    )
    cb.build_and_init()
    logger = get_logger("multi")
    assert_emitted(logger, "INFO", "emit", "level and name filters both satisfied")
    assert_suppressed(logger, "DEBUG", "suppress", "record below the configured level")


@pytest.mark.parametrize(
    ("first_filter", "second_filter"),
    [
        (
            ("lvl", LevelFilterBuilder().with_max_level("DEBUG")),
            None,
        ),
        (
            ("lvl", LevelFilterBuilder().with_max_level("DEBUG")),
            ("name", NameFilterBuilder().with_prefix("core")),
        ),
    ],
    ids=["remove_all_filters", "replace_with_name_filter"],
)
def test_reconfig_replaces_filters(
    first_filter: tuple[str, LevelFilterBuilder],
    second_filter: tuple[str, NameFilterBuilder] | None,
) -> None:
    """Reconfiguring should replace or drop filters as requested."""
    cb = (
        ConfigBuilder()
        .with_filter(first_filter[0], first_filter[1])
        .with_logger("core", LoggerConfigBuilder().with_filters([first_filter[0]]))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
    )
    cb.build_and_init()
    logger = get_logger("core")
    assert_suppressed(logger, "ERROR", "drop", "initial max-level DEBUG filter")

    reconfig = ConfigBuilder().with_root_logger(
        LoggerConfigBuilder().with_level("DEBUG")
    )
    if second_filter is not None:
        reconfig = reconfig.with_filter(second_filter[0], second_filter[1]).with_logger(
            "core", LoggerConfigBuilder().with_filters([second_filter[0]])
        )
    else:
        reconfig = reconfig.with_logger("core", LoggerConfigBuilder())
    reconfig.build_and_init()

    assert_emitted(
        get_logger("core"),
        "ERROR",
        "emit",
        "reconfiguration must discard the previous max-level filter",
    )


def test_reconfig_with_unknown_filter_preserves_previous_filters() -> None:
    """Unknown filters should leave previous filters intact."""
    cb = (
        ConfigBuilder()
        .with_filter("lvl", LevelFilterBuilder().with_max_level("DEBUG"))
        .with_logger("core", LoggerConfigBuilder().with_filters(["lvl"]))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
    )
    cb.build_and_init()
    logger = get_logger("core")
    assert_suppressed(logger, "ERROR", "drop", "initial max-level DEBUG filter")

    bad = (
        ConfigBuilder()
        .with_logger("core", LoggerConfigBuilder().with_filters(["missing"]))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
    )
    with pytest.raises(KeyError, match="missing"):
        bad.build_and_init()

    assert_suppressed(
        get_logger("core"),
        "ERROR",
        "still drop",
        "a rejected configuration must not disturb the installed filters",
    )


def test_filter_clearing() -> None:
    """Clearing filters should re-enable previously suppressed records."""
    cb = (
        ConfigBuilder()
        .with_filter("lvl", LevelFilterBuilder().with_max_level("DEBUG"))
        .with_logger("core", LoggerConfigBuilder().with_filters(["lvl"]))
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
    )
    cb.build_and_init()
    logger = get_logger("core")
    assert_suppressed(logger, "INFO", "drop", "max-level DEBUG filter is installed")
    logger.clear_filters()
    assert_emitted(logger, "INFO", "emit", "filters cleared at runtime")


def test_multiple_filters_clearing() -> None:
    """Clearing multiple filters should restore emissions across checks."""
    cb = (
        ConfigBuilder()
        .with_filter("lvl", LevelFilterBuilder().with_max_level("DEBUG"))
        .with_filter("name", NameFilterBuilder().with_prefix("other"))
        .with_logger(
            "core",
            LoggerConfigBuilder().with_level("DEBUG").with_filters(["lvl", "name"]),
        )
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
    )
    cb.build_and_init()
    logger = get_logger("core")
    assert_suppressed(logger, "ERROR", "blocked by level filter", "max-level DEBUG")
    assert_suppressed(logger, "DEBUG", "blocked by name", "name prefix 'other'")
    logger.clear_filters()
    assert_emitted(logger, "ERROR", "allowed now", "both filters cleared")
    assert_emitted(logger, "DEBUG", "also allowed", "both filters cleared")
