import pytest
from pytest_bdd import given, when, then, scenarios, parsers

from femtologging import (
    ConfigBuilder,
    FormatterBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
)


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


@when(parsers.parse('I add stream handler "{hid}"'))
def add_stream_handler(config_builder: ConfigBuilder, hid: str) -> None:
    handler = StreamHandlerBuilder.stderr()
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
def configuration_matches_snapshot(config_builder: ConfigBuilder, snapshot) -> None:
    assert config_builder.as_dict() == snapshot
    config_builder.build_and_init()


@when("I set version 2")
def set_version(config_builder: ConfigBuilder) -> None:
    config_builder.with_version(2)


@then("building the configuration fails")
def build_fails(config_builder: ConfigBuilder) -> None:
    with pytest.raises(ValueError):
        config_builder.build_and_init()


scenarios("features/config_builder.feature")


def test_duplicate_formatter_overwrites() -> None:
    """Second formatter with same ID should replace the first."""
    builder = ConfigBuilder()
    fmt1 = FormatterBuilder().with_format("one")
    fmt2 = FormatterBuilder().with_format("two")
    builder.with_formatter("fmt", fmt1)
    builder.with_formatter("fmt", fmt2)
    config = builder.as_dict()
    assert config["formatters"]["fmt"]["format"] == "two", (
        "Later formatter should overwrite earlier one"
    )


def test_duplicate_logger_overwrites() -> None:
    """Second logger with same ID should replace the first."""
    builder = ConfigBuilder()
    logger1 = LoggerConfigBuilder().with_level("INFO")
    logger2 = LoggerConfigBuilder().with_level("ERROR")
    builder.with_logger("core", logger1)
    builder.with_logger("core", logger2)
    builder.with_root_logger(LoggerConfigBuilder().with_level("WARNING"))
    config = builder.as_dict()
    assert config["loggers"]["core"]["level"] == "ERROR", (
        "Later logger should overwrite earlier one"
    )


def test_logger_config_builder_optional_fields_set() -> None:
    """Test that optional fields are included when explicitly set."""
    logger = (
        LoggerConfigBuilder()
        .with_level("DEBUG")
        .with_propagate(False)
        .with_filters(["myfilter"])
        .with_handlers(["console", "file"])
    )
    config = logger.as_dict()
    assert config["level"] == "DEBUG", "Level should be included when set"
    assert config["propagate"] is False, "Propagate should be included when set"
    assert config["filters"] == ["myfilter"], "Filters should be included when set"
    assert config["handlers"] == ["console", "file"], (
        "Handlers should be included when set"
    )


def test_logger_config_builder_optional_fields_omitted() -> None:
    """Test that optional fields are omitted when not set."""
    logger = LoggerConfigBuilder().with_level("WARNING")
    config = logger.as_dict()
    assert config["level"] == "WARN", "Level should be normalised to WARN"
    assert "propagate" not in config, "Propagate should be omitted when not set"
    assert "filters" not in config, "Filters should be omitted when not set"
    assert "handlers" not in config, "Handlers should be omitted when not set"


def test_no_root_logger_behavior() -> None:
    """Test that building without a root logger raises ValueError."""
    builder = ConfigBuilder()
    with pytest.raises(ValueError):
        builder.build_and_init()


def test_unknown_handler_id_raises_value_error() -> None:
    """Building with an unknown handler identifier fails."""
    builder = ConfigBuilder()
    logger = LoggerConfigBuilder().with_handlers(["missing"])
    builder.with_logger("core", logger)
    builder.with_root_logger(LoggerConfigBuilder().with_level("INFO"))
    with pytest.raises(ValueError):
        builder.build_and_init()


@pytest.mark.parametrize(
    ("first", "second", "expected"),
    [
        ("INFO", "ERROR", "ERROR"),
        ("ERROR", "INFO", "INFO"),
        ("DEBUG", "WARN", "WARN"),
    ],
    ids=["INFO→ERROR", "ERROR→INFO", "DEBUG→WARN"],
)
def test_root_logger_last_assignment_wins(
    first: str, second: str, expected: str
) -> None:
    """Verify last-write-wins semantics when assigning the root logger multiple times."""
    builder = ConfigBuilder()
    builder.with_root_logger(LoggerConfigBuilder().with_level(first))
    builder.with_root_logger(LoggerConfigBuilder().with_level(second))
    config = builder.as_dict()
    assert config["root"]["level"] == expected, (
        f"Last root logger assignment wins: {first}→{second}"
    )
