from __future__ import annotations

import collections.abc as cabc
import pytest
from pytest_bdd import given, parsers, scenarios, then, when
from syrupy import SnapshotAssertion

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    LevelFilterBuilder,
    NameFilterBuilder,
    get_logger,
    reset_manager,
    FilterBuildError,
)


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    reset_manager()
    yield
    reset_manager()


@given("a ConfigBuilder", target_fixture="config_builder")
def config_builder() -> ConfigBuilder:
    return ConfigBuilder()


@when(parsers.parse('I add stream handler "{hid}" targeting "{target}"'))
def add_stream_handler(config_builder: ConfigBuilder, hid: str, target: str) -> None:
    handler = (
        StreamHandlerBuilder.stderr()
        if target.lower() == "stderr"
        else StreamHandlerBuilder.stdout()
    )
    config_builder.with_handler(hid, handler)


@when(parsers.parse('I add level filter "{fid}" with max level "{level}"'))
def add_level_filter(config_builder: ConfigBuilder, fid: str, level: str) -> None:
    filt = LevelFilterBuilder().with_max_level(level)
    config_builder.with_filter(fid, filt)


@when(parsers.parse('I add name filter "{fid}" with prefix "{prefix}"'))
def add_name_filter(config_builder: ConfigBuilder, fid: str, prefix: str) -> None:
    filt = NameFilterBuilder().with_prefix(prefix)
    config_builder.with_filter(fid, filt)


@when(
    parsers.parse(
        'I add logger "{name}" with handler "{handler}" and filter "{filter_id}"'
    )
)
def add_logger_with_filter(
    config_builder: ConfigBuilder, name: str, handler: str, filter_id: str
) -> None:
    logger = LoggerConfigBuilder().with_handlers([handler]).with_filters([filter_id])
    config_builder.with_logger(name, logger)


@when(parsers.parse('I add logger "{name}" with filter "{filter_id}"'))
def add_logger_only_filter(
    config_builder: ConfigBuilder, name: str, filter_id: str
) -> None:
    logger = LoggerConfigBuilder().with_filters([filter_id])
    config_builder.with_logger(name, logger)


@when(parsers.parse('I set root logger with level "{level}"'))
def set_root(config_builder: ConfigBuilder, level: str) -> None:
    root = LoggerConfigBuilder().with_level(level)
    config_builder.with_root_logger(root)


@then("the configuration matches snapshot")
def configuration_matches_snapshot(
    config_builder: ConfigBuilder, snapshot: SnapshotAssertion
) -> None:
    assert config_builder.as_dict() == snapshot
    config_builder.build_and_init()


@then(parsers.parse('logger "{name}" emits "{level}"'))
def logger_emits(name: str, level: str) -> None:
    logger = get_logger(name)
    assert logger.log(level, "msg") is not None


@then(parsers.parse('logger "{name}" suppresses "{level}"'))
def logger_suppresses(name: str, level: str) -> None:
    logger = get_logger(name)
    assert logger.log(level, "msg") is None


@then("building the configuration fails")
def build_fails(config_builder: ConfigBuilder) -> None:
    with pytest.raises(FilterBuildError):
        config_builder.build_and_init()


@then(parsers.parse('building the configuration fails with error containing "{msg}"'))
def build_fails_with_message(config_builder: ConfigBuilder, msg: str) -> None:
    with pytest.raises(FilterBuildError) as excinfo:
        config_builder.build_and_init()
    assert msg in str(excinfo.value)


@then(
    parsers.parse(
        'building the configuration fails with value error containing "{msg}"'
    )
)
def build_fails_with_value_error(config_builder: ConfigBuilder, msg: str) -> None:
    with pytest.raises(ValueError) as excinfo:
        config_builder.build_and_init()
    assert msg in str(excinfo.value)


scenarios("features/filters.feature")


def test_logger_with_multiple_filters() -> None:
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
    with pytest.raises(ValueError):
        bad.build_and_init()

    logger_after = get_logger("core")
    assert logger_after.log("ERROR", "still drop") is None


def test_filter_clearing() -> None:
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
