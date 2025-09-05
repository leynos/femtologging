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
    with pytest.raises((FilterBuildError, ValueError)):
        config_builder.build_and_init()


@then(parsers.parse('building the configuration fails with error containing "{msg}"'))
def build_fails_with_message(config_builder: ConfigBuilder, msg: str) -> None:
    with pytest.raises((FilterBuildError, ValueError)) as excinfo:
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
