"""BDD steps for logger propagation behaviour scenarios."""

from __future__ import annotations

import dataclasses
import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    ConfigBuilder,
    FileHandlerBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
    reset_manager,
)

if typ.TYPE_CHECKING:
    from syrupy import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "propagate.feature"))


@dataclasses.dataclass(slots=True)
class HandlerContext:
    """Hold handler state and log file path for a scenario."""

    builder: FileHandlerBuilder
    path: Path


@dataclasses.dataclass(slots=True)
class PropagateContext:
    """Hold propagation test state for a scenario."""

    config_builder: ConfigBuilder
    handlers: dict[str, HandlerContext]
    loggers: set[str]


@pytest.fixture
def propagate_ctx() -> PropagateContext:
    """Create a fresh propagation context for each scenario."""
    return PropagateContext(
        config_builder=ConfigBuilder(),
        handlers={},
        loggers=set(),
    )


@given("a clean manager state")
def given_clean_manager() -> None:
    """Reset the global logger manager to a clean state."""
    reset_manager()


@given(
    parsers.parse('a file handler "{hid}" writing to a temporary file'),
    target_fixture="propagate_ctx",
)
def given_file_handler(
    propagate_ctx: PropagateContext, hid: str, tmp_path: Path
) -> PropagateContext:
    """Create a file handler writing to a temporary file."""
    path = tmp_path / f"{hid}.log"
    builder = FileHandlerBuilder(str(path)).with_flush_after_records(1)
    propagate_ctx.handlers[hid] = HandlerContext(builder=builder, path=path)
    propagate_ctx.config_builder.with_handler(hid, builder)
    return propagate_ctx


@given(
    parsers.parse(
        'a ConfigBuilder with root logger using handler "{hid}" at level "{level}"'
    ),
    target_fixture="propagate_ctx",
)
def given_root_with_handler(
    propagate_ctx: PropagateContext, hid: str, level: str
) -> PropagateContext:
    """Configure root logger with specified handler and level."""
    root = LoggerConfigBuilder().with_level(level).with_handlers([hid])
    propagate_ctx.config_builder.with_root_logger(root)
    return propagate_ctx


def _add_logger_without_handlers(
    propagate_ctx: PropagateContext, name: str, level: str
) -> PropagateContext:
    """Add a logger without handlers to the configuration."""
    logger_cfg = LoggerConfigBuilder().with_level(level)
    propagate_ctx.config_builder.with_logger(name, logger_cfg)
    propagate_ctx.loggers.add(name)
    return propagate_ctx


@given(
    parsers.parse('a child logger "{name}" at level "{level}" without handlers'),
    target_fixture="propagate_ctx",
)
def given_child_no_handlers(
    propagate_ctx: PropagateContext, name: str, level: str
) -> PropagateContext:
    """Configure a child logger with no handlers (relies on propagation)."""
    return _add_logger_without_handlers(propagate_ctx, name, level)


@given(
    parsers.parse(
        'an intermediate logger "{name}" at level "{level}" without handlers'
    ),
    target_fixture="propagate_ctx",
)
def given_intermediate_logger_no_handlers(
    propagate_ctx: PropagateContext, name: str, level: str
) -> PropagateContext:
    """Configure an intermediate (non-leaf, non-root) logger without handlers."""
    return _add_logger_without_handlers(propagate_ctx, name, level)


@given(
    parsers.parse('a leaf logger "{name}" at level "{level}" without handlers'),
    target_fixture="propagate_ctx",
)
def given_leaf_logger_no_handlers(
    propagate_ctx: PropagateContext, name: str, level: str
) -> PropagateContext:
    """Configure a leaf logger (deepest in hierarchy) without handlers."""
    return _add_logger_without_handlers(propagate_ctx, name, level)


@given(
    parsers.parse('a child logger "{name}" at level "{level}" without propagation'),
    target_fixture="propagate_ctx",
)
def given_child_propagate_disabled(
    propagate_ctx: PropagateContext, name: str, level: str
) -> PropagateContext:
    """Configure a child logger with propagation disabled."""
    propagate = False
    child = LoggerConfigBuilder().with_level(level).with_propagate(propagate)
    propagate_ctx.config_builder.with_logger(name, child)
    propagate_ctx.loggers.add(name)
    return propagate_ctx


@given(
    parsers.parse('a child logger "{name}" at level "{level}" using handler "{hid}"'),
    target_fixture="propagate_ctx",
)
def given_child_with_handler(
    propagate_ctx: PropagateContext, name: str, level: str, hid: str
) -> PropagateContext:
    """Configure a child logger with its own handler."""
    child = LoggerConfigBuilder().with_level(level).with_handlers([hid])
    propagate_ctx.config_builder.with_logger(name, child)
    propagate_ctx.loggers.add(name)
    return propagate_ctx


# Snapshot scenario steps - using distinct fixture and step patterns
@given("a ConfigBuilder for snapshot test", target_fixture="config_builder")
def given_config_builder() -> ConfigBuilder:
    """Create a fresh ConfigBuilder instance for snapshot tests."""
    return ConfigBuilder()


@given(parsers.parse('a stream handler "{hid}" targeting "{target}"'))
def given_stream_handler(config_builder: ConfigBuilder, hid: str, target: str) -> None:
    """Add a stream handler to the config builder."""
    handler = (
        StreamHandlerBuilder.stderr()
        if target.lower() == "stderr"
        else StreamHandlerBuilder.stdout()
    )
    config_builder.with_handler(hid, handler)


@given(parsers.parse('a logger "{name}" at level "{level}" with propagate true'))
def given_logger_propagate_enabled(
    config_builder: ConfigBuilder, name: str, level: str
) -> None:
    """Configure a logger with propagation explicitly enabled."""
    propagate = True
    child = LoggerConfigBuilder().with_level(level).with_propagate(propagate)
    config_builder.with_logger(name, child)


@given(parsers.parse('a logger "{name}" at level "{level}" with propagate false'))
def given_logger_propagate_disabled(
    config_builder: ConfigBuilder, name: str, level: str
) -> None:
    """Configure a logger with propagation disabled."""
    propagate = False
    child = LoggerConfigBuilder().with_level(level).with_propagate(propagate)
    config_builder.with_logger(name, child)


@given(parsers.parse('a root logger at level "{level}"'))
def given_root_logger(config_builder: ConfigBuilder, level: str) -> None:
    """Configure the root logger."""
    root = LoggerConfigBuilder().with_level(level)
    config_builder.with_root_logger(root)


@when("I build and initialise the configuration")
def when_build_and_init(propagate_ctx: PropagateContext) -> None:
    """Build and initialise the logging configuration."""
    propagate_ctx.config_builder.build_and_init()


@when(parsers.parse('I log "{message}" at level "{level}" from logger "{name}"'))
def when_log_message(message: str, level: str, name: str) -> None:
    """Log a message from the specified logger."""
    logger = get_logger(name)
    logger.log(level, message)


@when("I flush all loggers")
def when_flush_all(propagate_ctx: PropagateContext) -> None:
    """Flush all loggers to ensure records are written."""
    # Flush all tracked child loggers first
    for name in propagate_ctx.loggers:
        logger = get_logger(name)
        logger.flush_handlers()
    # Always flush root last to ensure propagated records are written
    root = get_logger("root")
    root.flush_handlers()


@when(parsers.parse('I disable propagation on logger "{name}"'))
def when_disable_propagation(name: str) -> None:
    """Disable propagation on the specified logger at runtime."""
    logger = get_logger(name)
    logger.set_propagate(False)


@when(parsers.parse('I enable propagation on logger "{name}"'))
def when_enable_propagation(name: str) -> None:
    """Enable propagation on the specified logger at runtime."""
    logger = get_logger(name)
    logger.set_propagate(True)


def _read_handler_file(propagate_ctx: PropagateContext, hid: str) -> str:
    """Read the contents of a handler's log file."""
    handler_ctx = propagate_ctx.handlers.get(hid)
    if handler_ctx is None:
        msg = f"No handler context found for '{hid}'"
        raise KeyError(msg)
    return handler_ctx.path.read_text() if handler_ctx.path.exists() else ""


@then(parsers.parse('the root handler file contains "{text}"'))
def then_root_contains(propagate_ctx: PropagateContext, text: str) -> None:
    """Assert that the root handler file contains the specified text."""
    contents = _read_handler_file(propagate_ctx, "root_handler")
    assert text in contents, f"Expected '{text}' in root handler file, got: {contents}"


@then(parsers.parse('the root handler file does not contain "{text}"'))
def then_root_not_contains(propagate_ctx: PropagateContext, text: str) -> None:
    """Assert that the root handler file does not contain the specified text."""
    contents = _read_handler_file(propagate_ctx, "root_handler")
    assert text not in contents, (
        f"Expected '{text}' NOT in root handler file, got: {contents}"
    )


@then(parsers.parse('the child handler file contains "{text}"'))
def then_child_contains(propagate_ctx: PropagateContext, text: str) -> None:
    """Assert that the child handler file contains the specified text."""
    contents = _read_handler_file(propagate_ctx, "child_handler")
    assert text in contents, f"Expected '{text}' in child handler file, got: {contents}"


@then("the configuration matches propagate enabled snapshot")
def then_propagate_enabled_snapshot(
    config_builder: ConfigBuilder, snapshot: SnapshotAssertion
) -> None:
    """Assert that configuration with propagate=true matches snapshot."""
    config = config_builder.as_dict()
    assert config == snapshot


@then("the configuration matches propagate disabled snapshot")
def then_propagate_disabled_snapshot(
    config_builder: ConfigBuilder, snapshot: SnapshotAssertion
) -> None:
    """Assert that configuration with propagate=false matches snapshot."""
    config = config_builder.as_dict()
    assert config == snapshot
