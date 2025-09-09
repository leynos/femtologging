import pytest
from pytest_bdd import given, when, then, scenarios, parsers
from syrupy import SnapshotAssertion

from femtologging import (
    ConfigBuilder,
    FormatterBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
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
    config_builder.build_and_init()


@when("I set version 2")
def set_version(config_builder: ConfigBuilder) -> None:
    config_builder.with_version(2)


@then("building the configuration fails")
def build_fails(config_builder: ConfigBuilder) -> None:
    with pytest.raises(ValueError):
        config_builder.build_and_init()


@then(parsers.parse('building the configuration fails with error containing "{msg}"'))
def build_fails_with_value_error(config_builder: ConfigBuilder, msg: str) -> None:
    with pytest.raises(ValueError) as excinfo:
        config_builder.build_and_init()
    assert msg in str(excinfo.value)


@then(
    parsers.parse('building the configuration fails with key error containing "{msg}"')
)
def build_fails_with_key_error(config_builder: ConfigBuilder, msg: str) -> None:
    with pytest.raises(KeyError) as excinfo:
        config_builder.build_and_init()
    assert msg in str(excinfo.value)


@then(parsers.parse('loggers "{first}" and "{second}" share handler "{hid}"'))
def loggers_share_handler(first: str, second: str, hid: str) -> None:
    del hid  # parameter required by step signature
    first_logger = get_logger(first)
    second_logger = get_logger(second)
    h1 = first_logger.handler_ptrs_for_test()
    h2 = second_logger.handler_ptrs_for_test()
    assert h1[0] == h2[0], "handlers should be shared"


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


def test_duplicate_handler_overwrites() -> None:
    """Second handler with same ID should replace the first."""
    builder = ConfigBuilder()
    handler1 = StreamHandlerBuilder.stderr()
    handler2 = StreamHandlerBuilder.stdout()
    builder.with_handler("console", handler1)
    builder.with_handler("console", handler2)
    logger = LoggerConfigBuilder().with_handlers(["console"])
    builder.with_logger("core", logger)
    builder.with_root_logger(LoggerConfigBuilder().with_level("INFO"))
    config = builder.as_dict()
    assert config["handlers"]["console"]["target"] == "stdout", (
        "Later handler should overwrite earlier one",
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


def test_unknown_handler_id_raises_key_error() -> None:
    """Building with an unknown handler identifier raises KeyError."""
    builder = ConfigBuilder()
    logger = LoggerConfigBuilder().with_handlers(["missing"])
    builder.with_logger("core", logger)
    builder.with_root_logger(LoggerConfigBuilder().with_level("INFO"))
    with pytest.raises(KeyError, match="missing"):
        builder.build_and_init()


def test_disable_existing_loggers_clears_unmentioned() -> None:
    """Loggers not present in new config are disabled."""
    handler = StreamHandlerBuilder.stderr()
    root = LoggerConfigBuilder().with_level("INFO")
    builder = (
        ConfigBuilder()
        .with_handler("h", handler)
        .with_root_logger(root)
        .with_logger("stale", LoggerConfigBuilder().with_handlers(["h"]))
    )
    builder.build_and_init()

    stale = get_logger("stale")
    assert stale.handler_ptrs_for_test(), "stale logger should have a handler"

    rebuild = (
        ConfigBuilder()
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        .with_disable_existing_loggers(True)
    )
    rebuild.build_and_init()

    stale = get_logger("stale")
    assert stale.handler_ptrs_for_test() == [], "stale logger should be disabled"


def test_disable_existing_loggers_keeps_ancestors() -> None:
    """Ancestor loggers remain active when child logger is kept."""
    handler = StreamHandlerBuilder.stderr()
    parent_builder = (
        ConfigBuilder()
        .with_handler("h", handler)
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        .with_logger("parent", LoggerConfigBuilder().with_handlers(["h"]))
    )
    parent_builder.build_and_init()

    parent = get_logger("parent")
    assert parent.handler_ptrs_for_test(), "parent should have a handler"

    rebuild = (
        ConfigBuilder()
        .with_handler("h", StreamHandlerBuilder.stderr())
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        .with_logger("parent.child", LoggerConfigBuilder().with_handlers(["h"]))
        .with_disable_existing_loggers(True)
    )
    rebuild.build_and_init()

    parent = get_logger("parent")
    assert parent.handler_ptrs_for_test(), "ancestor logger should remain active"


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
