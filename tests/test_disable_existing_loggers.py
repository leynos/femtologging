"""Regression tests for ``disable_existing_loggers`` transitions."""

import collections.abc as cabc
import pathlib

import pytest

from femtologging import (
    ConfigBuilder,
    FileHandlerBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
)

type BuilderFactory = cabc.Callable[[], ConfigBuilder]

HANDLER_ID = "h"
DISABLE_EXISTING_LOGGERS = True


@pytest.fixture
def new_info_stderr_builder() -> BuilderFactory:
    """Return builders with an INFO root logger and a shared stderr handler."""

    def factory() -> ConfigBuilder:
        return (
            ConfigBuilder()
            .with_handler(HANDLER_ID, StreamHandlerBuilder.stderr())
            .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        )

    return factory


def test_disable_existing_loggers_clears_unmentioned(
    new_info_stderr_builder: BuilderFactory,
) -> None:
    """Loggers not present in new config are disabled and lose attachments."""
    builder = new_info_stderr_builder().with_logger(
        "stale", LoggerConfigBuilder().with_handlers([HANDLER_ID])
    )
    builder.build_and_init()

    stale = get_logger("stale")
    assert stale.handler_ptrs_for_test(), "stale logger should have a handler"

    rebuild = (
        ConfigBuilder()
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        .with_disable_existing_loggers(DISABLE_EXISTING_LOGGERS)
    )
    rebuild.build_and_init()

    stale = get_logger("stale")
    assert not stale.handler_ptrs_for_test(), (
        "stale logger should lose handlers after it is omitted"
    )
    assert stale.disabled, (
        "an omitted logger must be flagged disabled, not merely stripped of handlers"
    )


def test_disabled_logger_cannot_reach_configured_root_handler(
    new_info_stderr_builder: BuilderFactory, tmp_path: pathlib.Path
) -> None:
    """Default propagation cannot bypass an omitted logger's disabled flag."""
    builder = new_info_stderr_builder().with_logger(
        "omitted", LoggerConfigBuilder().with_level("INFO")
    )
    builder.build_and_init()
    omitted = get_logger("omitted")
    assert omitted.propagate, "the omitted logger should start with propagation enabled"

    log_path = tmp_path / "root.log"
    rebuild = (
        ConfigBuilder()
        .with_handler("root_file", FileHandlerBuilder(str(log_path)))
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["root_file"])
        )
        .with_disable_existing_loggers(DISABLE_EXISTING_LOGGERS)
    )
    rebuild.build_and_init()

    assert omitted.disabled, "the omitted logger should be disabled"
    disabled_drops = omitted.get_disabled_drops()
    assert omitted.log("ERROR", "must not escape") is None, (
        "a disabled logger must reject the record instead of dispatching it"
    )
    assert omitted.get_disabled_drops() == disabled_drops + 1, (
        "the rejected record should be counted as disabled traffic"
    )
    assert omitted.flush_handlers(), "the omitted logger should flush its (empty) queue"
    assert get_logger("root").flush_handlers(), "the root handler should flush"
    contents = log_path.read_text() if log_path.exists() else ""
    assert "must not escape" not in contents, (
        "an omitted logger must not reach the configured root handler"
    )


def test_explicitly_named_logger_is_reenabled(
    new_info_stderr_builder: BuilderFactory,
) -> None:
    """Naming a logger again must clear the disabled flag."""
    builder = new_info_stderr_builder().with_logger(
        "core", LoggerConfigBuilder().with_handlers([HANDLER_ID])
    )
    builder.build_and_init()

    (
        ConfigBuilder()
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        .with_disable_existing_loggers(DISABLE_EXISTING_LOGGERS)
    ).build_and_init()
    assert get_logger("core").disabled, "core should be disabled while omitted"

    rebuild = new_info_stderr_builder().with_logger(
        "core", LoggerConfigBuilder().with_level("DEBUG").with_handlers([HANDLER_ID])
    )
    rebuild.build_and_init()

    core = get_logger("core")
    assert not core.disabled, "naming the logger again should re-enable it"
    assert core.isEnabledFor("DEBUG"), "the re-enabled logger should honour its level"


@pytest.mark.parametrize(
    "ancestors",
    [["parent"], ["grandparent", "grandparent.parent"]],
    ids=["parent", "grandparent"],
)
def test_disable_existing_loggers_keeps_ancestors(
    new_info_stderr_builder: BuilderFactory, ancestors: list[str]
) -> None:
    """Configured descendants preserve ancestor handlers."""
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
    rebuild = (
        new_info_stderr_builder()
        .with_logger(child_name, LoggerConfigBuilder().with_handlers([HANDLER_ID]))
        .with_disable_existing_loggers(DISABLE_EXISTING_LOGGERS)
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
