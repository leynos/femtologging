import pathlib

import pytest

import femtologging
from femtologging import (
    ConfigBuilder,
    FormatterBuilder,
    LoggerConfigBuilder,
    RotatingFileHandlerBuilder,
    StreamHandlerBuilder,
    get_logger,
)


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


def test_rotating_handler_supported(tmp_path: pathlib.Path) -> None:
    """ConfigBuilder should accept rotating file handler builders."""
    builder = ConfigBuilder().with_disable_existing_loggers(True)
    log_path = tmp_path / "rotating.log"
    rotating = (
        RotatingFileHandlerBuilder(str(log_path))
        .with_max_bytes(1024)
        .with_backup_count(3)
    )
    builder.with_handler("rot", rotating)
    builder.with_root_logger(LoggerConfigBuilder().with_handlers(["rot"]))

    # Building should succeed and preserve the rotating handler configuration.
    builder.build_and_init()
    config = builder.as_dict()
    assert config["handlers"]["rot"]["path"] == str(log_path)
    assert config["handlers"]["rot"]["max_bytes"] == 1024
    assert config["handlers"]["rot"]["backup_count"] == 3


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


def make_info_stderr_builder() -> ConfigBuilder:
    """Create a builder with an INFO root logger and stderr handler."""
    return (
        ConfigBuilder()
        .with_handler("h", StreamHandlerBuilder.stderr())
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
    )


def make_builder_with_logger(logger_name: str) -> ConfigBuilder:
    """Create a builder with INFO root logger, stderr handler, and a named logger."""
    return make_info_stderr_builder().with_logger(
        logger_name, LoggerConfigBuilder().with_handlers(["h"])
    )


def test_disable_existing_loggers_clears_unmentioned() -> None:
    """Loggers not present in new config are disabled."""
    builder = make_builder_with_logger("stale")
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


@pytest.mark.parametrize(
    "ancestors",
    [
        ["parent"],
        ["grandparent", "grandparent.parent"],
    ],
    ids=["parent", "grandparent"],
)
def test_disable_existing_loggers_keeps_ancestors(ancestors: list[str]) -> None:
    """Ancestor loggers remain active when their descendants are configured."""
    builder = make_info_stderr_builder()
    for name in ancestors:
        builder = builder.with_logger(name, LoggerConfigBuilder().with_handlers(["h"]))
    builder.build_and_init()

    initial_handlers = {
        name: get_logger(name).handler_ptrs_for_test() for name in ancestors
    }

    child_name = f"{ancestors[-1]}.child"
    rebuild = (
        make_info_stderr_builder()
        .with_logger(child_name, LoggerConfigBuilder().with_handlers(["h"]))
        .with_disable_existing_loggers(True)
    )
    rebuild.build_and_init()

    child = get_logger(child_name)
    assert len(child.handler_ptrs_for_test()) == 1, "child should have one handler"
    for name in ancestors:
        assert get_logger(name).handler_ptrs_for_test() == initial_handlers[name], (
            "ancestor logger should retain its handler"
        )


@pytest.mark.parametrize(
    ("first", "second", "expected"),
    [
        ("INFO", "ERROR", "ERROR"),
        ("ERROR", "INFO", "INFO"),
        ("DEBUG", "WARN", "WARN"),
        ("INFO", "INFO", "INFO"),
    ],
    ids=["INFO→ERROR", "ERROR→INFO", "DEBUG→WARN", "INFO→INFO"],
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


def test_builder_symbols_exposed_publicly() -> None:
    """Builder classes must be reachable from both package and module namespaces."""
    import femtologging.config as config_module

    assert femtologging.ConfigBuilder is ConfigBuilder
    assert femtologging.LoggerConfigBuilder is LoggerConfigBuilder
    assert femtologging.FormatterBuilder is FormatterBuilder
    assert femtologging.StreamHandlerBuilder is StreamHandlerBuilder
    assert femtologging.RotatingFileHandlerBuilder is RotatingFileHandlerBuilder
    assert config_module.ConfigBuilder is ConfigBuilder
    assert config_module.LoggerConfigBuilder is LoggerConfigBuilder
