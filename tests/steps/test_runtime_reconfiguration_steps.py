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
    target = target.lower()
    if target == "stderr":
        builder = StreamHandlerBuilder.stderr()
    elif target == "stdout":
        builder = StreamHandlerBuilder.stdout()
    else:
        msg = f"unsupported stream target: {target}"
        raise ValueError(msg)
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


@then(parsers.parse('the runtime mutation fails with key error containing "{msg}"'))
def mutation_fails_with_key_error(
    runtime_mutation_error: BaseException, msg: str
) -> None:
    """Assert that the captured runtime mutation failure mentions the ID."""
    assert isinstance(runtime_mutation_error, KeyError), (
        f"expected KeyError but got {runtime_mutation_error!r}"
    )
    assert msg in str(runtime_mutation_error), (
        f"expected {msg!r} in {runtime_mutation_error!r}"
    )


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
        "handler_ids": handler_ids,
        "name": name,
        "level": logger.level,
        "propagate": logger.propagate,
        "handler_count": len(handler_ptrs),
    }
    assert state == snapshot


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
    logger = get_logger(name)
    assert len(logger.handler_ptrs_for_test()) == 0, (
        f"expected {name} to have no runtime handlers"
    )
