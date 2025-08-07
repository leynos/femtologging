import pytest
from pytest_bdd import given, when, then, scenarios

from femtologging import ConfigBuilder, FormatterBuilder, LoggerConfigBuilder

scenarios("features/config_builder.feature")


@given("a ConfigBuilder", target_fixture="config_builder")
def given_config_builder() -> ConfigBuilder:  # pragma: no cover - fixture
    return ConfigBuilder()


@when('I add formatter "fmt" with format "{level} {message}"')
def add_formatter(config_builder: ConfigBuilder) -> None:
    fmt = FormatterBuilder().with_format("{level} {message}")
    config_builder.add_formatter("fmt", fmt)


@when('I add logger "core" with level "INFO"')
def add_logger(config_builder: ConfigBuilder) -> None:
    logger = LoggerConfigBuilder().with_level("INFO")
    config_builder.add_logger("core", logger)


@when('I set root logger with level "WARN"')
def set_root(config_builder: ConfigBuilder) -> None:
    root = LoggerConfigBuilder().with_level("WARN")
    config_builder.set_root_logger(root)


@then("the configuration matches snapshot")
def configuration_matches_snapshot(config_builder: ConfigBuilder, snapshot) -> None:
    assert config_builder.as_dict() == snapshot
    config_builder.build_and_init()


@when("I set version 2")
def set_version(config_builder: ConfigBuilder) -> None:
    config_builder.version(2)


@then("building the configuration fails")
def build_fails(config_builder: ConfigBuilder) -> None:
    with pytest.raises(ValueError):
        config_builder.build_and_init()
