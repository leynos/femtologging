"""Unit tests for structured runtime reconfiguration workflows."""

from __future__ import annotations

import re
import typing as typ

import pytest

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

if typ.TYPE_CHECKING:
    import collections.abc as cabc

    from syrupy.assertion import SnapshotAssertion


# Fragment of the error raised when a root mutation is requested through both
# ``with_root_logger()`` and ``with_logger("root", ...)``.
ROOT_OVERLAP_MESSAGE = 'with_root_logger() and with_logger("root", ...)'


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    """Reset the global manager around each test."""
    reset_manager()
    yield
    reset_manager()


@pytest.fixture(name="core_config")
def fixture_core_config(reset_logger_state: None) -> None:
    """Install the baseline ``core`` logger used by the mutation tests.

    Depends on ``reset_logger_state`` so the configuration is installed after
    the manager has been reset, never before it.
    """
    (
        ConfigBuilder()
        .with_handler("stderr", StreamHandlerBuilder.stderr())
        .with_filter("lvl", LevelFilterBuilder().with_max_level("DEBUG"))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
        .with_logger(
            "core",
            LoggerConfigBuilder().with_handlers(["stderr"]).with_filters(["lvl"]),
        )
        .build_and_init()
    )


def test_runtime_builder_dict_matches_snapshot(snapshot: SnapshotAssertion) -> None:
    """The Python-facing runtime builder shape should stay stable."""
    runtime = (
        RuntimeConfigBuilder()
        .with_handler("stdout", StreamHandlerBuilder.stdout())
        .with_filter("name", NameFilterBuilder().with_prefix("core"))
        .with_root_logger(LoggerMutationBuilder().append_handlers(["stdout"]))
        .with_logger(
            "core",
            LoggerMutationBuilder()
            .with_level("ERROR")
            .replace_filters(["name"])
            .append_handlers(["stdout"]),
        )
    )
    assert runtime.as_dict() == snapshot, (
        "RuntimeConfigBuilder.as_dict() must keep the documented wire shape "
        "that dictConfig-style callers depend on"
    )


def test_runtime_apply_appends_handler_and_replaces_filters(
    core_config: None,
) -> None:
    """Happy-path runtime mutation should take effect immediately."""
    logger = get_logger("core")
    before = logger.handler_ptrs_for_test()

    (
        RuntimeConfigBuilder()
        .with_handler("stdout", StreamHandlerBuilder.stdout())
        .with_filter("name", NameFilterBuilder().with_prefix("core"))
        .with_logger(
            "core",
            LoggerMutationBuilder()
            .append_handlers(["stdout"])
            .replace_filters(["name"]),
        )
        .apply()
    )

    after = logger.handler_ptrs_for_test()
    assert after[: len(before)] == before, (
        "append_handlers must retain the pre-existing handlers in order, but "
        f"{after} does not start with {before}"
    )
    assert len(after) == len(before) + 1, (
        "append_handlers(['stdout']) must add exactly one handler, taking the "
        f"core logger from {len(before)} to {len(after)} handlers"
    )
    assert logger.log("ERROR", "now allowed") is not None, (
        "replace_filters(['name']) must swap out the DEBUG-only level filter "
        "so ERROR records reach the handlers"
    )


def test_runtime_apply_unknown_filter_preserves_previous_state(
    core_config: None,
) -> None:
    """Failed runtime mutation must leave the prior runtime configuration intact."""
    logger = get_logger("core")

    with pytest.raises(KeyError, match="missing"):
        (
            RuntimeConfigBuilder()
            .with_logger(
                "core",
                LoggerMutationBuilder().replace_filters(["missing"]),
            )
            .apply()
        )

    assert logger.log("ERROR", "still blocked") is None, (
        "expected logger.log('ERROR', ...) to stay blocked because the failed "
        "RuntimeConfigBuilder().with_logger('core', "
        "LoggerMutationBuilder().replace_filters(['missing'])).apply() must "
        "preserve the prior lvl filter"
    )


def test_runtime_apply_rejects_conflicting_collection_modes(
    core_config: None,
) -> None:
    """One logger mutation may not request multiple handler modes."""
    with pytest.raises(ValueError, match="multiple handlers mutation modes"):
        (
            RuntimeConfigBuilder()
            .with_logger(
                "core",
                LoggerMutationBuilder()
                .append_handlers(["stdout"])
                .replace_handlers(["stderr"]),
            )
            .apply()
        )


def test_runtime_apply_rejects_root_name_overlap(core_config: None) -> None:
    """Root mutations must not be provided through both root builder paths."""
    with pytest.raises(
        ValueError,
        match=re.escape(ROOT_OVERLAP_MESSAGE),
    ):
        (
            RuntimeConfigBuilder()
            .with_root_logger(LoggerMutationBuilder().with_level("ERROR"))
            .with_logger(
                "root", LoggerMutationBuilder().with_propagate(propagate=False)
            )
            .apply()
        )
