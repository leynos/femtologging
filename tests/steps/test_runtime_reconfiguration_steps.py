"""BDD steps for structured runtime reconfiguration scenarios."""

from __future__ import annotations

import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    ConfigBuilder,
    LevelFilterBuilder,
    LoggerConfigBuilder,
    LoggerMutationBuilder,
    NameFilterBuilder,
    RuntimeConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
    reset_manager,
)
from femtologging._rust_compat import _runtime_attachment_state_for_test

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy.assertion import SnapshotAssertion

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "runtime_reconfiguration.feature"))

_STREAM_HANDLER_FACTORIES = {
    "stderr": StreamHandlerBuilder.stderr,
    "stdout": StreamHandlerBuilder.stdout,
}


def _stream_handler_builder(target: str) -> StreamHandlerBuilder:
    """Build the configured stream handler, preserving the useful error."""
    normalized_target = target.lower()
    factory = _STREAM_HANDLER_FACTORIES.get(normalized_target)
    if factory is None:
        msg = f"unsupported stream target: {normalized_target}"
        raise ValueError(msg)
    return factory()


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    """Reset the global manager around each scenario."""
    reset_manager()
    yield
    reset_manager()


@given(
    parsers.parse('a runtime-configured logger named "{name}"'),
    target_fixture="runtime_logger_name",
)
def runtime_configured_logger(name: str) -> str:
    """Create a logger with one handler and one suppressing filter."""
    (
        ConfigBuilder()
        .with_handler("stderr", StreamHandlerBuilder.stderr())
        .with_filter("lvl", LevelFilterBuilder().with_max_level("DEBUG"))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
        .with_logger(
            name,
            LoggerConfigBuilder().with_handlers(["stderr"]).with_filters(["lvl"]),
        )
        .build_and_init()
    )
    return name


@when(
    parsers.parse(
        'I append runtime handler "{hid}" targeting "{target}" to logger "{name}"'
    )
)
def append_runtime_handler(hid: str, target: str, name: str) -> None:
    """Append a new stream handler to the named logger."""
    builder = _stream_handler_builder(target)
    (
        RuntimeConfigBuilder()
        .with_handler(hid, builder)
        .with_logger(name, LoggerMutationBuilder().append_handlers([hid]))
        .apply()
    )


@when(
    parsers.parse(
        'I replace logger "{name}" filters with name filter "{fid}" '
        'using prefix "{prefix}"'
    )
)
def replace_runtime_filters(name: str, fid: str, prefix: str) -> None:
    """Replace the logger filters with a name-prefix filter."""
    (
        RuntimeConfigBuilder()
        .with_filter(fid, NameFilterBuilder().with_prefix(prefix))
        .with_logger(name, LoggerMutationBuilder().replace_filters([fid]))
        .apply()
    )


@when(
    parsers.parse('I try to replace logger "{name}" filters with missing id "{fid}"'),
    target_fixture="runtime_mutation_error",
)
def replace_runtime_filters_with_missing(name: str, fid: str) -> BaseException:
    """Capture a failed runtime mutation for later assertions."""
    with pytest.raises(KeyError) as excinfo:
        (
            RuntimeConfigBuilder()
            .with_logger(name, LoggerMutationBuilder().replace_filters([fid]))
            .apply()
        )
    return excinfo.value


@when(
    parsers.parse(
        'I try to define unattached runtime handler "{hid}" targeting "{target}"'
    ),
    target_fixture="runtime_mutation_error",
)
def define_unattached_runtime_handler(hid: str, target: str) -> BaseException:
    """Capture the rejection of a handler definition no logger attaches."""
    builder = _stream_handler_builder(target)
    with pytest.raises(ValueError, match="unreferenced definitions") as excinfo:
        RuntimeConfigBuilder().with_handler(hid, builder).apply()
    return excinfo.value


@when(parsers.parse('I set root logger level to "{level}" via runtime mutation'))
def set_root_level(level: str) -> None:
    """Update the root logger level through the runtime control plane."""
    RuntimeConfigBuilder().with_root_logger(
        LoggerMutationBuilder().with_level(level)
    ).apply()


@then(parsers.parse('logger "{name}" has {count:d} handlers'))
def logger_has_handler_count(name: str, count: int) -> None:
    """Assert the handler count seen by the named logger."""
    actual = len(get_logger(name).handler_ptrs_for_test())
    assert actual == count, (
        f"expected {name} to have {count} handlers but found {actual}"
    )


@then(parsers.parse('logger "{name}" emits "{level}"'))
def logger_emits(name: str, level: str) -> None:
    """Assert that the logger emits at the requested level."""
    actual = get_logger(name).log(level, "emit")
    assert actual is not None, f"expected logger {name} to emit at level {level}"


@then(parsers.parse('logger "{name}" suppresses "{level}"'))
def logger_suppresses(name: str, level: str) -> None:
    """Assert that the logger suppresses at the requested level."""
    actual = get_logger(name).log(level, "suppress")
    assert actual is None, f"expected logger {name} to suppress level {level}"


def _assert_mutation_failed_with(
    error: BaseException, expected_type: type[BaseException], msg: str
) -> None:
    """Assert a captured mutation failure has the expected type and message."""
    assert isinstance(error, expected_type), (
        f"expected {expected_type.__name__} but got {error!r}"
    )
    assert msg in str(error), f"expected {msg!r} in {error!r}"


@then(parsers.parse('the runtime mutation fails with key error containing "{msg}"'))
def mutation_fails_with_key_error(
    runtime_mutation_error: BaseException, msg: str
) -> None:
    """Assert that the captured runtime mutation failure mentions the ID."""
    _assert_mutation_failed_with(runtime_mutation_error, KeyError, msg)


@then(parsers.parse('the runtime mutation fails with value error containing "{msg}"'))
def mutation_fails_with_value_error(
    runtime_mutation_error: BaseException, msg: str
) -> None:
    """Assert that the captured runtime mutation failure mentions the reason."""
    _assert_mutation_failed_with(runtime_mutation_error, ValueError, msg)


@then(parsers.parse('logger "{name}" runtime state matches snapshot'))
def runtime_state_matches_snapshot(name: str, snapshot: SnapshotAssertion) -> None:
    """Assert that a normalized runtime state payload stays stable."""
    logger = get_logger(name)
    attachment_state = _runtime_attachment_state_for_test(name)
    handler_ids, filter_ids = attachment_state or ([], [])
    handler_ptrs = logger.handler_ptrs_for_test()
    assert len(handler_ptrs) == len(handler_ids), (
        f"expected resolved handler count for {name} to match runtime metadata: "
        f"{len(handler_ptrs)} != {len(handler_ids)}"
    )
    state = {
        "filter_ids": filter_ids,
        "filter_count": len(filter_ids),
        "handler_ids": handler_ids,
        "name": name,
        "level": logger.level,
        "propagate": logger.propagate,
        "handler_count": len(handler_ptrs),
    }
    assert state == snapshot, (
        f"runtime state for {name} must match the recorded snapshot; got {state!r}"
    )


@when(parsers.parse('logger "{name}" runtime handlers and filters are cleared'))
def clear_runtime_handlers_and_filters(name: str) -> None:
    """Clear all runtime handlers and filters for the given logger."""
    (
        RuntimeConfigBuilder()
        .with_logger(
            name,
            LoggerMutationBuilder().clear_handlers().clear_filters(),
        )
        .apply()
    )


@then(parsers.parse('logger "{name}" has no runtime handlers'))
def logger_has_no_runtime_handlers(name: str) -> None:
    """Assert that the logger has no runtime handlers after mutation."""
    remaining = get_logger(name).handler_ptrs_for_test()
    assert not remaining, (
        f"clearing runtime handlers must leave {name} with none, "
        f"but {len(remaining)} remain"
    )
