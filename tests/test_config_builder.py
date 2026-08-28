"""Unit tests covering ConfigBuilder and related builder utilities."""

import collections.abc as cabc
import pathlib

import pytest
from hypothesis import given
from hypothesis import strategies as st

import femtologging
from femtologging import (
    ConfigBuilder,
    FormatterBuilder,
    LoggerConfigBuilder,
    RotatingFileHandlerBuilder,
    StreamHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
    get_logger,
)
from femtologging import config as config_module

type BuilderFactory = cabc.Callable[[], ConfigBuilder]

# Identifier used for the shared stderr handler in reconfiguration tests.
HANDLER_ID = "h"

# Every level spelling accepted by the Rust parser, mapped to its canonical
# rendering in the emitted configuration dictionary.
LEVEL_ALIASES = {
    "TRACE": "TRACE",
    "DEBUG": "DEBUG",
    "INFO": "INFO",
    "WARN": "WARN",
    "WARNING": "WARN",
    "ERROR": "ERROR",
    "CRITICAL": "CRITICAL",
}

# Builder classes that must be re-exported from the top-level package.
PACKAGE_BUILDER_EXPORTS = {
    "ConfigBuilder": ConfigBuilder,
    "FormatterBuilder": FormatterBuilder,
    "LoggerConfigBuilder": LoggerConfigBuilder,
    "RotatingFileHandlerBuilder": RotatingFileHandlerBuilder,
    "StreamHandlerBuilder": StreamHandlerBuilder,
    "TimedRotatingFileHandlerBuilder": TimedRotatingFileHandlerBuilder,
}

# Subset of the above that ``femtologging.config`` must also expose directly.
CONFIG_MODULE_EXPORTS = ("ConfigBuilder", "LoggerConfigBuilder")


@pytest.fixture
def new_info_stderr_builder() -> BuilderFactory:
    """Return a factory for builders with an INFO root logger and stderr handler.

    A factory rather than a single builder is returned because several tests
    need a second, independent configuration to reconfigure the manager with.

    Returns
    -------
    BuilderFactory
        Callable returning a freshly configured :class:`ConfigBuilder`.
    """

    def factory() -> ConfigBuilder:
        return (
            ConfigBuilder()
            .with_handler(HANDLER_ID, StreamHandlerBuilder.stderr())
            .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        )

    return factory


def assert_handler_fields(
    handler: cabc.Mapping[str, object],
    expected: cabc.Mapping[str, object],
    context: str,
) -> None:
    """Assert that a serialized handler carries the expected configuration.

    Parameters
    ----------
    handler:
        Serialized handler entry taken from ``ConfigBuilder.as_dict()``.
    expected:
        Field values the builder was asked to record.
    context:
        Short description of the handler under test, used in failure messages.
    """
    for key, value in expected.items():
        actual = handler.get(key)
        assert actual == value, (
            f"{context}: builder should record {key!r} as {value!r}, got {actual!r}"
        )


def assign_root_levels(first: str, second: str) -> str:
    """Assign the root logger twice and return the level that survives.

    Parameters
    ----------
    first:
        Level for the discarded first root logger assignment.
    second:
        Level for the final root logger assignment.

    Returns
    -------
    str
        Root level recorded in the serialized configuration.
    """
    builder = ConfigBuilder()
    builder.with_root_logger(LoggerConfigBuilder().with_level(first))
    builder.with_root_logger(LoggerConfigBuilder().with_level(second))
    return builder.as_dict()["root"]["level"]


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
        "Later handler should overwrite earlier one"
    )


def test_rotating_handler_supported(tmp_path: pathlib.Path) -> None:
    """ConfigBuilder should accept rotating file handler builders."""
    disable_existing = True
    builder = ConfigBuilder().with_disable_existing_loggers(disable_existing)
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
    assert_handler_fields(
        builder.as_dict()["handlers"]["rot"],
        {"path": str(log_path), "max_bytes": 1024, "backup_count": 3},
        "rotating file handler",
    )


def test_timed_rotating_handler_supported(tmp_path: pathlib.Path) -> None:
    """ConfigBuilder should accept timed rotating file handler builders."""
    builder = ConfigBuilder()
    log_path = tmp_path / "timed.log"
    use_utc = True
    timed = (
        TimedRotatingFileHandlerBuilder(str(log_path))
        .with_when("MIDNIGHT")
        .with_interval(2)
        .with_backup_count(4)
        .with_utc(use_utc)
    )
    builder.with_handler("timed", timed)
    builder.with_root_logger(LoggerConfigBuilder().with_handlers(["timed"]))

    builder.build_and_init()
    handler = builder.as_dict()["handlers"]["timed"]
    assert_handler_fields(
        handler,
        {
            "path": str(log_path),
            "when": "MIDNIGHT",
            "interval": 2,
            "backup_count": 4,
        },
        "timed rotating file handler",
    )
    assert handler["utc"] is True, (
        "timed rotating file handler should record UTC as a true boolean, "
        f"got {handler['utc']!r}"
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
    propagate_flag = False
    logger = (
        LoggerConfigBuilder()
        .with_level("DEBUG")
        .with_propagate(propagate_flag)
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
    assert config["level"] == "WARN", "Level should be normalized to WARN"
    assert "propagate" not in config, "Propagate should be omitted when not set"
    assert "filters" not in config, "Filters should be omitted when not set"
    assert "handlers" not in config, "Handlers should be omitted when not set"


def test_build_without_root_logger_raises() -> None:
    """Test that building without a root logger raises ValueError."""
    builder = ConfigBuilder()
    with pytest.raises(ValueError, match="root logger configuration"):
        builder.build_and_init()


def test_unknown_handler_id_raises_key_error() -> None:
    """Building with an unknown handler identifier raises KeyError."""
    builder = ConfigBuilder()
    logger = LoggerConfigBuilder().with_handlers(["missing"])
    builder.with_logger("core", logger)
    builder.with_root_logger(LoggerConfigBuilder().with_level("INFO"))
    with pytest.raises(KeyError, match="missing"):
        builder.build_and_init()


def test_disable_existing_loggers_clears_unmentioned(
    new_info_stderr_builder: BuilderFactory,
) -> None:
    """Loggers not present in new config are disabled."""
    builder = new_info_stderr_builder().with_logger(
        "stale", LoggerConfigBuilder().with_handlers([HANDLER_ID])
    )
    builder.build_and_init()

    stale = get_logger("stale")
    assert stale.handler_ptrs_for_test(), "stale logger should have a handler"

    disable_existing = True
    rebuild = (
        ConfigBuilder()
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        .with_disable_existing_loggers(disable_existing)
    )
    rebuild.build_and_init()

    stale = get_logger("stale")
    assert not stale.handler_ptrs_for_test(), (
        "stale logger should be disabled once it is absent from the new config"
    )


@pytest.mark.parametrize(
    "ancestors",
    [
        ["parent"],
        ["grandparent", "grandparent.parent"],
    ],
    ids=["parent", "grandparent"],
)
def test_disable_existing_loggers_keeps_ancestors(
    new_info_stderr_builder: BuilderFactory, ancestors: list[str]
) -> None:
    """Ancestor loggers remain active when their descendants are configured."""
    builder = new_info_stderr_builder()
    for name in ancestors:
        builder = builder.with_logger(
            name, LoggerConfigBuilder().with_handlers([HANDLER_ID])
        )
    builder.build_and_init()

    initial_handlers = {
        name: get_logger(name).handler_ptrs_for_test() for name in ancestors
    }

    child_name = f"{ancestors[-1]}.child"
    disable_existing = True
    rebuild = (
        new_info_stderr_builder()
        .with_logger(child_name, LoggerConfigBuilder().with_handlers([HANDLER_ID]))
        .with_disable_existing_loggers(disable_existing)
    )
    rebuild.build_and_init()

    child = get_logger(child_name)
    assert len(child.handler_ptrs_for_test()) == 1, (
        f"newly configured logger {child_name!r} should have exactly one handler"
    )
    for name in ancestors:
        assert get_logger(name).handler_ptrs_for_test() == initial_handlers[name], (
            f"ancestor logger {name!r} should retain its handler when a "
            "descendant is configured"
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
    """Verify last-write-wins semantics when assigning the root logger."""
    assert assign_root_levels(first, second) == expected, (
        f"Last root logger assignment wins: {first}→{second}"
    )


@given(
    first=st.sampled_from(sorted(LEVEL_ALIASES)),
    second=st.sampled_from(sorted(LEVEL_ALIASES)),
)
def test_root_logger_last_assignment_wins_for_any_level_pair(
    first: str, second: str
) -> None:
    """Last-write-wins should hold for every accepted level spelling."""
    expected = LEVEL_ALIASES[second]
    assert assign_root_levels(first, second) == expected, (
        f"Root level after {first}→{second} should be the canonical form of "
        f"the last assignment ({expected})"
    )


@pytest.mark.parametrize("name", sorted(PACKAGE_BUILDER_EXPORTS))
def test_builder_symbol_exposed_on_package(name: str) -> None:
    """Builder classes must be reachable from the top-level package namespace."""
    assert getattr(femtologging, name, None) is PACKAGE_BUILDER_EXPORTS[name], (
        f"femtologging.{name} must be the builder class importable from the package"
    )


@pytest.mark.parametrize("name", CONFIG_MODULE_EXPORTS)
def test_builder_symbol_exposed_on_config_module(name: str) -> None:
    """Core builder classes must also be reachable from ``femtologging.config``."""
    assert getattr(config_module, name, None) is PACKAGE_BUILDER_EXPORTS[name], (
        f"femtologging.config.{name} must be the same object as femtologging.{name}"
    )
