"""Unit tests for structured runtime reconfiguration workflows."""

from __future__ import annotations

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


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    """Reset the global manager around each test."""
    reset_manager()
    yield
    reset_manager()


def _configure_core_logger() -> None:
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
    assert runtime.as_dict() == snapshot


def test_runtime_apply_appends_handler_and_replaces_filters() -> None:
    """Happy-path runtime mutation should take effect immediately."""
    _configure_core_logger()
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
    assert len(after) == 2
    assert after[0] == before[0]
    assert logger.log("ERROR", "now allowed") is not None


def test_runtime_apply_unknown_filter_preserves_previous_state() -> None:
    """Failed runtime mutation must leave the prior runtime configuration intact."""
    _configure_core_logger()
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

    assert logger.log("ERROR", "still blocked") is None


def test_runtime_apply_rejects_conflicting_collection_modes() -> None:
    """One logger mutation may not request multiple handler modes."""
    _configure_core_logger()

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
