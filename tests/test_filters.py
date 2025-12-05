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
    reset_manager,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    """Reset the global logging manager around each test."""
    reset_manager()
    yield
    reset_manager()


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
    assert logger.log("INFO", "emit") is not None
    assert logger.log("DEBUG", "suppress") is None


@pytest.mark.parametrize(
    ("_scenario", "first_filter", "second_filter"),
    [
        (
            "remove_all_filters",
            ("lvl", LevelFilterBuilder().with_max_level("DEBUG")),
            None,
        ),
        (
            "replace_with_name_filter",
            ("lvl", LevelFilterBuilder().with_max_level("DEBUG")),
            ("name", NameFilterBuilder().with_prefix("core")),
        ),
    ],
)
def test_reconfig_replaces_filters(
    _scenario: str,
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
    assert logger.log("ERROR", "drop") is None

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

    logger_after = get_logger("core")
    assert logger_after.log("ERROR", "emit") is not None


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
    assert logger.log("ERROR", "drop") is None

    bad = (
        ConfigBuilder()
        .with_logger("core", LoggerConfigBuilder().with_filters(["missing"]))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
    )
    with pytest.raises(KeyError, match="missing"):
        bad.build_and_init()

    logger_after = get_logger("core")
    assert logger_after.log("ERROR", "still drop") is None


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
    assert logger.log("INFO", "drop") is None
    logger.clear_filters()
    assert logger.log("INFO", "emit") is not None


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
    assert logger.log("ERROR", "blocked by level filter") is None
    assert logger.log("DEBUG", "blocked by name") is None
    logger.clear_filters()
    assert logger.log("ERROR", "allowed now") is not None
    assert logger.log("DEBUG", "also allowed") is not None
