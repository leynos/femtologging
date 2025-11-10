from __future__ import annotations

import copy
import sys
from dataclasses import dataclass
from typing import Any

import pytest
from pytest_bdd import given, parsers, scenarios, then, when
from syrupy import SnapshotAssertion

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    basicConfig,
    dictConfig,
    get_logger,
    reset_manager,
)

scenarios("features/config_compat.feature")


@dataclass
class ConfigExample:
    builder: ConfigBuilder
    dict_schema: dict[str, Any]


@pytest.fixture
def captured_outputs() -> dict[str, str | None]:
    """Store captured log outputs for comparison."""
    return {}


@given("the logging system is reset")
def reset_logging() -> None:
    reset_manager()


@given("a canonical configuration example", target_fixture="config_example")
def config_example() -> ConfigExample:
    builder = (
        ConfigBuilder()
        .with_handler("console", StreamHandlerBuilder.stderr())
        .with_logger(
            "worker",
            LoggerConfigBuilder().with_handlers(["console"]).with_propagate(False),
        )
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["console"])
        )
    )
    dict_schema = {
        "version": 1,
        "handlers": {"console": {"class": "femtologging.StreamHandler"}},
        "loggers": {"worker": {"handlers": ["console"], "propagate": False}},
        "root": {"handlers": ["console"], "level": "INFO"},
    }
    return ConfigExample(builder=builder, dict_schema=dict_schema)


@when("I apply the builder configuration")
def apply_builder_configuration(config_example: ConfigExample) -> None:
    config_example.builder.build_and_init()


@when("I apply the dictConfig schema")
def apply_dictconfig_schema(config_example: ConfigExample) -> None:
    dictConfig(copy.deepcopy(config_example.dict_schema))


@when("I drop the root logger from the dictConfig schema")
def drop_root_logger(config_example: ConfigExample) -> None:
    config_example.dict_schema.pop("root", None)


@when("I reset the logging system")
def reset_logging_when() -> None:
    reset_manager()


@when(parsers.parse('I call basicConfig with level "{level}" and stream stdout'))
def call_basic_config(level: str) -> None:
    basicConfig(level=level, stream=sys.stdout, force=True)


@when(
    parsers.parse('I log "{message}" at "{level}" capturing as "{key}"'),
)
def log_and_capture(
    message: str,
    level: str,
    key: str,
    captured_outputs: dict[str, str | None],
) -> None:
    logger = get_logger("root")
    captured_outputs[key] = logger.log(level, message)


@then("the captured outputs match snapshot")
def outputs_match_snapshot(
    captured_outputs: dict[str, str | None],
    snapshot: SnapshotAssertion,
) -> None:
    assert captured_outputs == snapshot


@then(parsers.parse('logging "{message}" at "{level}" from root matches snapshot'))
def log_matches_snapshot(
    message: str,
    level: str,
    snapshot: SnapshotAssertion,
) -> None:
    logger = get_logger("root")
    assert logger.log(level, message) == snapshot


@then(parsers.parse('applying the schema via dictConfig fails with "{msg}"'))
def schema_application_fails(config_example: ConfigExample, msg: str) -> None:
    with pytest.raises(ValueError, match=msg):
        dictConfig(copy.deepcopy(config_example.dict_schema))
