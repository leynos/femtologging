"""BDD steps for the ConfigBuilder Gherkin scenarios."""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

if typ.TYPE_CHECKING:
    from syrupy import SnapshotAssertion

from femtologging import (
    ConfigBuilder,
    FormatterBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
)

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "config_builder.feature"))


@given("a ConfigBuilder", target_fixture="config_builder")
def given_config_builder() -> ConfigBuilder:
    return ConfigBuilder()


@when('I add formatter "fmt" with format "{level} {message}"')
def add_formatter(config_builder: ConfigBuilder) -> None:
    fmt = FormatterBuilder().with_format("{level} {message}")
    config_builder.with_formatter("fmt", fmt)


@when('I add logger "core" with level "INFO"')
def add_logger(config_builder: ConfigBuilder) -> None:
    logger = LoggerConfigBuilder().with_level("INFO")
    config_builder.with_logger("core", logger)


@when(parsers.parse('I add stream handler "{hid}" targeting "{target}"'))
def add_stream_handler(config_builder: ConfigBuilder, hid: str, target: str) -> None:
    handler = (
        StreamHandlerBuilder.stderr()
        if target.lower() == "stderr"
        else StreamHandlerBuilder.stdout()
    )
    config_builder.with_handler(hid, handler)


@when(parsers.parse('I add logger "{name}" with handler "{handler}"'))
def add_logger_with_handler(
    config_builder: ConfigBuilder, name: str, handler: str
) -> None:
    logger = LoggerConfigBuilder().with_handlers([handler])
    config_builder.with_logger(name, logger)


@when(
    parsers.parse('I add logger "{name}" with level "{level}" and handler "{handler}"')
)
def add_logger_with_level_and_handler(
    config_builder: ConfigBuilder, name: str, level: str, handler: str
) -> None:
    logger = LoggerConfigBuilder().with_level(level).with_handlers([handler])
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


@then("the configuration is built and initialised")
def configuration_is_built(config_builder: ConfigBuilder) -> None:
    config_builder.build_and_init()


@when("I set version 2")
def set_version(config_builder: ConfigBuilder) -> None:
    config_builder.with_version(2)


@then("building the configuration fails")
def build_fails(config_builder: ConfigBuilder) -> None:
    with pytest.raises(ValueError, match="unsupported configuration version"):
        config_builder.build_and_init()


@then(parsers.parse('building the configuration fails with error containing "{msg}"'))
def build_fails_with_value_error(config_builder: ConfigBuilder, msg: str) -> None:
    with pytest.raises(ValueError, match=msg):
        config_builder.build_and_init()


@then(
    parsers.parse('building the configuration fails with key error containing "{msg}"')
)
def build_fails_with_key_error(config_builder: ConfigBuilder, msg: str) -> None:
    with pytest.raises(KeyError, match=msg):
        config_builder.build_and_init()


@then(parsers.parse('loggers "{first}" and "{second}" share handler "{hid}"'))
def loggers_share_handler(first: str, second: str, hid: str) -> None:
    _ = hid  # step parameter required by signature
    first_logger = get_logger(first)
    second_logger = get_logger(second)
    h1 = first_logger.handler_ptrs_for_test()
    h2 = second_logger.handler_ptrs_for_test()
    assert h1[0] == h2[0], "handlers should be shared"
