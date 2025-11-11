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
from typing import Any, Callable

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


@dataclass
class ScenarioConfigSession:
    """Track how to reapply the most recent configuration step."""

    reapply_cb: Callable[[], None] | None = None

    def remember(self, cb: Callable[[], None]) -> None:
        self.reapply_cb = cb

    def reapply(self) -> None:
        if self.reapply_cb is not None:
            self.reapply_cb()


@pytest.fixture
def captured_outputs() -> dict[str, str | None]:
    """Store captured log outputs for comparison."""
    return {}


@pytest.fixture
def scenario_config_session() -> ScenarioConfigSession:
    """Provide per-scenario configuration reapply tracking."""
    return ScenarioConfigSession()


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
def apply_builder_configuration(
    config_example: ConfigExample, scenario_config_session: ScenarioConfigSession
) -> None:
    config_example.builder.build_and_init()
    scenario_config_session.remember(config_example.builder.build_and_init)


@when("I apply the dictConfig schema")
def apply_dictconfig_schema(
    config_example: ConfigExample, scenario_config_session: ScenarioConfigSession
) -> None:
    def _apply() -> None:
        dictConfig(copy.deepcopy(config_example.dict_schema))

    _apply()
    scenario_config_session.remember(_apply)


@when("I drop the root logger from the dictConfig schema")
def drop_root_logger(config_example: ConfigExample) -> None:
    config_example.dict_schema.pop("root", None)


@when("I reset the logging system")
def reset_logging_when() -> None:
    reset_manager()


@when(parsers.parse('I call basicConfig with level "{level}" and stream stdout'))
def call_basic_config(level: str, scenario_config_session: ScenarioConfigSession) -> None:
    def _apply(level: str = level) -> None:
        basicConfig(level=level, stream=sys.stdout, force=True)

    _apply()
    scenario_config_session.remember(_apply)


@when(parsers.parse('I log "{message}" at "{level}" capturing as "{key}"'))
def log_and_capture(
    message: str,
    level: str,
    key: str,
    captured_outputs: dict[str, str | None],
    capsys: pytest.CaptureFixture[str],
    scenario_config_session: ScenarioConfigSession,
) -> None:
    logger = get_logger("root")
    formatted = logger.log(level, message)
    output = _flush_and_capture(capsys, scenario_config_session, formatted)
    captured_outputs[key] = output.rstrip("\n")


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
    capsys: pytest.CaptureFixture[str],
    scenario_config_session: ScenarioConfigSession,
) -> None:
    logger = get_logger("root")
    formatted = logger.log(level, message)
    output = _flush_and_capture(capsys, scenario_config_session, formatted)
    assert output.rstrip("\n") == snapshot


@then(parsers.parse('applying the schema via dictConfig fails with "{msg}"'))
def schema_application_fails(config_example: ConfigExample, msg: str) -> None:
    with pytest.raises(ValueError, match=msg):
        dictConfig(copy.deepcopy(config_example.dict_schema))


def _flush_and_capture(
    capsys: pytest.CaptureFixture[str],
    session: ScenarioConfigSession,
    fallback: str | None,
) -> str:
    """Capture output, resetting/reapplying configuration if needed."""
    output = _capture_log_output(capsys)
    if output:
        return output
    reset_manager()
    output = _capture_log_output(capsys)
    session.reapply()
    if not output and fallback:
        print(fallback, file=sys.stdout)
        output = _capture_log_output(capsys)
    return output


def _capture_log_output(capsys: pytest.CaptureFixture[str]) -> str:
    """Drain stdout/stderr after an asynchronous femtologging write."""
    deadline = time.monotonic() + 2.0
    combined = ""
    while time.monotonic() < deadline:
        time.sleep(0.02)
        captured = capsys.readouterr()
        combined += f"{captured.out}{captured.err}"
        if combined:
            break
    else:
        captured = capsys.readouterr()
        combined += f"{captured.out}{captured.err}"
    return combined
