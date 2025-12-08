"""BDD steps for dynamic log level update scenarios."""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "dynamic_level.feature"))


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    from femtologging import reset_manager

    reset_manager()
    yield
    reset_manager()


@given("a ConfigBuilder", target_fixture="config_builder")
def config_builder() -> ConfigBuilder:
    return ConfigBuilder()


@given(parsers.parse('I add stream handler "{hid}" targeting "{target}"'))
@when(parsers.parse('I add stream handler "{hid}" targeting "{target}"'))
def add_stream_handler(config_builder: ConfigBuilder, hid: str, target: str) -> None:
    handler = (
        StreamHandlerBuilder.stderr()
        if target.lower() == "stderr"
        else StreamHandlerBuilder.stdout()
    )
    config_builder.with_handler(hid, handler)


@given(parsers.parse('I set root logger with level "{level}"'))
@when(parsers.parse('I set root logger with level "{level}"'))
def set_root(config_builder: ConfigBuilder, level: str) -> None:
    root = LoggerConfigBuilder().with_level(level)
    config_builder.with_root_logger(root)


@given("the configuration is built and initialised")
@then("the configuration is built and initialised")
def configuration_is_built(config_builder: ConfigBuilder) -> None:
    config_builder.build_and_init()


@when(parsers.parse('I set logger "{name}" level to "{level}"'))
def set_logger_level(name: str, level: str) -> None:
    logger = get_logger(name)
    logger.set_level(level)


@then(parsers.parse('logger "{name}" level is "{level}"'))
def logger_level_is(name: str, level: str) -> None:
    logger = get_logger(name)
    assert logger.level == level


@then(parsers.parse('logger "{name}" emits "{level}"'))
def logger_emits(name: str, level: str) -> None:
    logger = get_logger(name)
    assert logger.log(level, "msg") is not None


@then(parsers.parse('logger "{name}" suppresses "{level}"'))
def logger_suppresses(name: str, level: str) -> None:
    logger = get_logger(name)
    assert logger.log(level, "msg") is None


@then(parsers.parse('logger "{name}" level state matches snapshot'))
def logger_level_matches_snapshot(name: str, snapshot: SnapshotAssertion) -> None:
    logger = get_logger(name)
    assert {"name": name, "level": logger.level} == snapshot
