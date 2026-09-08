"""BDD steps for filter configuration and evaluation scenarios."""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    ConfigBuilder,
    FilterBuildError,
    LevelFilterBuilder,
    LoggerConfigBuilder,
    NameFilterBuilder,
    PythonCallbackFilterBuilder,
    StreamHandlerBuilder,
    get_logger,
    reset_manager,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "filters.feature"))


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    """Isolate each scenario by clearing global logging state around it."""
    reset_manager()
    yield
    reset_manager()


@given("a ConfigBuilder", target_fixture="config_builder")
def config_builder() -> ConfigBuilder:
    return ConfigBuilder()


@when(parsers.parse('I add stream handler "{hid}" targeting "{target}"'))
def add_stream_handler(config_builder: ConfigBuilder, hid: str, target: str) -> None:
    handler = (
        StreamHandlerBuilder.stderr()
        if target.lower() == "stderr"
        else StreamHandlerBuilder.stdout()
    )
    config_builder.with_handler(hid, handler)


@when(parsers.parse('I add level filter "{fid}" with max level "{level}"'))
def add_level_filter(config_builder: ConfigBuilder, fid: str, level: str) -> None:
    filt = LevelFilterBuilder().with_max_level(level)
    config_builder.with_filter(fid, filt)


@when(parsers.parse('I add name filter "{fid}" with prefix "{prefix}"'))
def add_name_filter(config_builder: ConfigBuilder, fid: str, prefix: str) -> None:
    filt = NameFilterBuilder().with_prefix(prefix)
    config_builder.with_filter(fid, filt)


@given(
    parsers.parse('a callback filter "{fid}" that records logger names'),
    target_fixture="handler_filter_observed",
)
def add_callback_filter(config_builder: ConfigBuilder, fid: str) -> list[str]:
    """Attach a callback filter that records each producer-thread logger name."""
    observed: list[str] = []

    def record_logger_name(record: object) -> bool:
        record_attributes = vars(record)
        observed.append(typ.cast("str", record_attributes["name"]))
        record_attributes["correlation_id"] = "REQ-99"
        return True

    config_builder.with_filter(fid, PythonCallbackFilterBuilder(record_logger_name))
    return observed


@when(parsers.parse('I add stream handler "{hid}" with filter "{filter_id}"'))
def add_stream_handler_with_filter(
    config_builder: ConfigBuilder, hid: str, filter_id: str
) -> None:
    """Attach one configured filter to a shared handler."""
    config_builder.with_handler(
        hid,
        StreamHandlerBuilder.stderr().with_filters([filter_id]),
    )


@when(
    parsers.parse(
        'I add logger "{name}" with handler "{handler}" and filter "{filter_id}"'
    )
)
def add_logger_with_filter(
    config_builder: ConfigBuilder, name: str, handler: str, filter_id: str
) -> None:
    logger = LoggerConfigBuilder().with_handlers([handler]).with_filters([filter_id])
    config_builder.with_logger(name, logger)


@when(parsers.parse('I add logger "{name}" with filter "{filter_id}"'))
def add_logger_only_filter(
    config_builder: ConfigBuilder, name: str, filter_id: str
) -> None:
    logger = LoggerConfigBuilder().with_filters([filter_id])
    config_builder.with_logger(name, logger)


@when(parsers.parse('I set root logger with level "{level}"'))
def set_root(config_builder: ConfigBuilder, level: str) -> None:
    root = LoggerConfigBuilder().with_level(level)
    config_builder.with_root_logger(root)


@when(parsers.parse('I set root logger with level "{level}" and handler "{handler}"'))
def set_root_with_handler(
    config_builder: ConfigBuilder, level: str, handler: str
) -> None:
    """Configure a root logger that routes records through one handler."""
    root = LoggerConfigBuilder().with_level(level).with_handlers([handler])
    config_builder.with_root_logger(root)


@then("the configuration matches snapshot")
def configuration_matches_snapshot(
    config_builder: ConfigBuilder, snapshot: SnapshotAssertion
) -> None:
    assert config_builder.as_dict() == snapshot, (
        "the filter configuration must match the recorded snapshot"
    )


@then("the configuration is built and initialized")
def configuration_is_built(config_builder: ConfigBuilder) -> None:
    config_builder.build_and_init()


@then(parsers.parse('logger "{name}" emits "{level}"'))
def logger_emits(name: str, level: str) -> None:
    logger = get_logger(name)
    assert logger.log(level, "msg") is not None, (
        f"configured filters must let logger '{name}' emit at level '{level}'"
    )


@then(parsers.parse('logger "{name}" suppresses "{level}"'))
def logger_suppresses(name: str, level: str) -> None:
    logger = get_logger(name)
    assert logger.log(level, "msg") is None, (
        f"configured filters must suppress level '{level}' on logger '{name}'"
    )


@then(parsers.parse('the callback filter observed "{first}" and "{second}"'))
def callback_filter_observed_loggers(
    handler_filter_observed: list[str], first: str, second: str
) -> None:
    """Verify a shared handler filter ran for both direct and propagated records."""
    assert handler_filter_observed == [first, second], (
        "handler filters must run for direct and propagated records"
    )


def _assert_build_error_mentions(
    config_builder: ConfigBuilder,
    expected_type: type[BaseException],
    fragment: str,
) -> None:
    """Build the configuration and assert it fails, naming ``fragment``.

    Parameters
    ----------
    config_builder : ConfigBuilder
        Builder whose ``build_and_init`` call is expected to fail.
    expected_type : type[BaseException]
        Exception type the failure must be reported as.
    fragment : str
        Substring the operator-facing error message must contain, so that
        misconfigured filters are diagnosable from the message alone.

    """
    with pytest.raises(expected_type) as excinfo:
        config_builder.build_and_init()
    rendered = str(excinfo.value)
    assert fragment in rendered, (
        f"{expected_type.__name__} from build_and_init must name '{fragment}' "
        f"so the misconfiguration is diagnosable, got: {rendered}"
    )


@then("building the configuration fails")
def build_fails(config_builder: ConfigBuilder) -> None:
    with pytest.raises(FilterBuildError):
        config_builder.build_and_init()


@then(parsers.parse('building the configuration fails with error containing "{msg}"'))
def build_fails_with_message(config_builder: ConfigBuilder, msg: str) -> None:
    _assert_build_error_mentions(config_builder, FilterBuildError, msg)


@then(
    parsers.parse('building the configuration fails with key error containing "{msg}"')
)
def build_fails_with_key_error(config_builder: ConfigBuilder, msg: str) -> None:
    _assert_build_error_mentions(config_builder, KeyError, msg)
