"""BDD step definitions covering builder/dictConfig/basicConfig compatibility."""

from __future__ import annotations

import copy
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING, Any

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    basicConfig,
    dictConfig,
    get_logger,
    reset_manager,
)

if TYPE_CHECKING:
    from syrupy import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "config_compat.feature"))


@dataclass(slots=True)
class ConfigExample:
    """Pair a builder instance with the equivalent dictConfig schema."""

    builder: ConfigBuilder
    dict_schema: dict[str, Any]


@dataclass(slots=True)
class LogCaptureContext:
    """Bundle captured file descriptor output and recorded values."""

    capfd: pytest.CaptureFixture[str]
    captured_outputs: dict[str, str | None]


@pytest.fixture
def captured_outputs() -> dict[str, str | None]:
    """Store captured log outputs for comparison."""
    return {}


@pytest.fixture
def log_capture_context(
    capfd: pytest.CaptureFixture[str],
    captured_outputs: dict[str, str | None],
) -> LogCaptureContext:
    """Provide combined capture context and output storage."""
    return LogCaptureContext(
        capfd=capfd,
        captured_outputs=captured_outputs,
    )


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


@when(parsers.parse('I log "{message}" at "{level}" capturing as "{key}"'))
def log_and_capture(
    message: str,
    level: str,
    key: str,
    log_capture_context: LogCaptureContext,
) -> None:
    output = _log_message_and_get_output(message, level, log_capture_context)
    log_capture_context.captured_outputs[key] = output


@then("the captured outputs match snapshot")
def outputs_match_snapshot(
    log_capture_context: LogCaptureContext,
    snapshot: SnapshotAssertion,
) -> None:
    assert log_capture_context.captured_outputs == snapshot, (
        "Captured outputs must match the expected snapshot"
    )


@then(parsers.parse('logging "{message}" at "{level}" from root matches snapshot'))
def log_matches_snapshot(
    message: str,
    level: str,
    snapshot: SnapshotAssertion,
    log_capture_context: LogCaptureContext,
) -> None:
    output = _log_message_and_get_output(message, level, log_capture_context)
    assert output == snapshot, "Root logger output must match the expected snapshot"


@then(parsers.parse('applying the schema via dictConfig fails with "{msg}"'))
def schema_application_fails(config_example: ConfigExample, msg: str) -> None:
    with pytest.raises(ValueError, match=msg):
        dictConfig(copy.deepcopy(config_example.dict_schema))


def _log_message_and_get_output(
    message: str,
    level: str,
    log_capture_context: LogCaptureContext,
) -> str:
    """Log ``message`` at ``level`` and return the captured output without trailing newlines."""
    logger = get_logger("root")
    _drain_fd_capture(log_capture_context.capfd)
    logger.log(level, message)
    _flush_root_handlers(logger)
    return _capture_handler_output(log_capture_context.capfd)


def _capture_handler_output(capfd: pytest.CaptureFixture[str]) -> str:
    """Return the flushed handler output without trailing newlines."""
    return _drain_fd_capture(capfd).rstrip("\n")


def _drain_fd_capture(capfd: pytest.CaptureFixture[str]) -> str:
    """Read and clear the captured stdout/stderr buffers (destroys their contents)."""
    captured = capfd.readouterr()
    return f"{captured.out}{captured.err}"


def _flush_root_handlers(logger) -> None:
    """Ensure femtologging's asynchronous handlers flush pending records."""
    flushed = logger.flush_handlers()
    if not flushed:
        pytest.fail("Root handlers failed to flush pending femtologging records")
