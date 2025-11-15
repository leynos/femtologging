"""Config compatibility step definitions.

This module provides pytest-bdd step implementations that exercise the builder,
dictConfig, and basicConfig flows for femtologging to ensure their outputs stay
in sync. Import these steps in Gherkin-based suites to verify compatibility
contracts end-to-end.

Examples
--------
The scenarios in ``tests/features/config_compat.feature`` use these steps to
compare builder output snapshots with dictConfig and basicConfig.
"""

from __future__ import annotations

import copy
import sys
import time
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


@dataclass(slots=True)
class ConfigExample:
    """Pair a builder instance with the equivalent dictConfig schema."""
    builder: ConfigBuilder
    dict_schema: dict[str, Any]  # Values may be heterogeneous across config schemas, so Any is intentional.


@dataclass(slots=True)
class LogCaptureContext:
    """Bundle capsys and output storage for cleaner function signatures."""

    capsys: pytest.CaptureFixture[str]
    captured_outputs: dict[str, str | None]


@pytest.fixture
def captured_outputs() -> dict[str, str | None]:
    """Store captured log outputs for comparison."""
    return {}


@pytest.fixture
def log_capture_context(
    capsys: pytest.CaptureFixture[str],
    captured_outputs: dict[str, str | None],
) -> LogCaptureContext:
    """Provide combined capture context and output storage."""
    return LogCaptureContext(
        capsys=capsys,
        captured_outputs=captured_outputs,
    )


@given("the logging system is reset")
def reset_logging() -> None:
    reset_manager()
    return


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
    return


@when("I apply the dictConfig schema")
def apply_dictconfig_schema(config_example: ConfigExample) -> None:
    dictConfig(copy.deepcopy(config_example.dict_schema))
    return


@when("I drop the root logger from the dictConfig schema")
def drop_root_logger(config_example: ConfigExample) -> None:
    config_example.dict_schema.pop("root", None)
    return


@when("I reset the logging system")
def reset_logging_when() -> None:
    reset_manager()
    return


@when(parsers.parse('I call basicConfig with level "{level}" and stream stdout'))
def call_basic_config(level: str) -> None:
    basicConfig(level=level, stream=sys.stdout, force=True)
    return


@when(parsers.parse('I log "{message}" at "{level}" capturing as "{key}"'))
def log_and_capture(
    message: str,
    level: str,
    key: str,
    log_capture_context: LogCaptureContext,
) -> None:
    output = _log_message_and_get_output(message, level, log_capture_context)
    log_capture_context.captured_outputs[key] = output
    return


@then("the captured outputs match snapshot")
def outputs_match_snapshot(
    log_capture_context: LogCaptureContext,
    snapshot: SnapshotAssertion,
) -> None:
    assert (
        log_capture_context.captured_outputs == snapshot
    ), "Captured outputs must match the expected snapshot"
    return


@then(parsers.parse('logging "{message}" at "{level}" from root matches snapshot'))
def log_matches_snapshot(
    message: str,
    level: str,
    snapshot: SnapshotAssertion,
    log_capture_context: LogCaptureContext,
) -> None:
    output = _log_message_and_get_output(message, level, log_capture_context)
    assert output == snapshot, "Root logger output must match the expected snapshot"
    return


@then(parsers.parse('applying the schema via dictConfig fails with "{msg}"'))
def schema_application_fails(config_example: ConfigExample, msg: str) -> None:
    with pytest.raises(ValueError, match=msg):
        dictConfig(copy.deepcopy(config_example.dict_schema))
    return


def _log_message_and_get_output(
    message: str,
    level: str,
    log_capture_context: LogCaptureContext,
) -> str:
    """Log ``message`` at ``level`` and return the captured output without trailing newlines."""
    logger = get_logger("root")
    # Drain any prior output so we only capture the new record.
    log_capture_context.capsys.readouterr()
    logger.log(level, message)
    return _capture_handler_output(log_capture_context.capsys)


def _capture_handler_output(capsys: pytest.CaptureFixture[str]) -> str:
    """Poll for handler output, accommodating async flush delays."""
    deadline = time.monotonic() + 2.0
    combined = ""
    while time.monotonic() < deadline:
        captured = capsys.readouterr()
        combined = f"{captured.out}{captured.err}"
        if combined:
            break
        time.sleep(0.02)
    else:
        captured = capsys.readouterr()
        combined = f"{captured.out}{captured.err}"
    return combined.rstrip("\n")
