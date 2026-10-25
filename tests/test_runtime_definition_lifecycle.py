"""Tests for runtime definition reachability and lifecycle."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import (
    ConfigBuilder,
    FileHandlerBuilder,
    HTTPHandlerBuilder,
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
    from pathlib import Path

# Fragment of the error raised when a transaction declares a definition that
# no logger in the resulting attachment graph references.
UNREFERENCED_DEFINITION_MESSAGE = "unreferenced definitions"

# Churn rounds make registry growth visible while keeping the suite fast.
CHURN_ROUNDS = 32


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    """Reset the global manager around each test."""
    reset_manager()
    yield
    reset_manager()


@pytest.fixture(name="core_config")
def fixture_core_config(reset_logger_state: None) -> None:
    """Install the baseline core logger used by lifecycle tests."""
    (
        ConfigBuilder()
        .with_handler("stderr", StreamHandlerBuilder.stderr())
        .with_filter("lvl", LevelFilterBuilder().with_max_level("DEBUG"))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
        .with_logger(
            "core",
            LoggerConfigBuilder()
            .with_level("DEBUG")
            .with_handlers(["stderr"])
            .with_filters(["lvl"]),
        )
        .build_and_init()
    )


def test_runtime_apply_rejects_unreferenced_handler(
    core_config: None, tmp_path: Path
) -> None:
    """A handler no logger attaches must be rejected before it is built.

    The rejection happens ahead of construction, so the assertion that the
    file was never created is what distinguishes "rejected up front" from
    "built and then dropped".
    """
    orphan = tmp_path / "orphan.log"

    with pytest.raises(ValueError, match=UNREFERENCED_DEFINITION_MESSAGE) as excinfo:
        RuntimeConfigBuilder().with_handler(
            "orphan", FileHandlerBuilder(str(orphan))
        ).apply()

    assert "orphan" in str(excinfo.value), (
        "the unreferenced-definition error must name the offending handler id "
        f"so the caller can find it; got {excinfo.value}"
    )
    assert not orphan.exists(), (
        f"rejecting {orphan} must happen before construction, so no file may "
        "be created for a handler that can never be referenced"
    )


def test_runtime_apply_rejects_unreferenced_filter(core_config: None) -> None:
    """A filter no logger attaches must be rejected, named under ``filters``."""
    with pytest.raises(ValueError, match=UNREFERENCED_DEFINITION_MESSAGE) as excinfo:
        (
            RuntimeConfigBuilder()
            .with_filter("orphan", NameFilterBuilder().with_prefix("core"))
            .apply()
        )

    message = str(excinfo.value)
    assert "filters" in message, (
        "the error must report unreferenced filters separately from handlers; "
        f"got {message}"
    )
    assert "orphan" in message, (
        f"the error must name the offending filter id; got {message}"
    )


def test_runtime_apply_rejects_unreferenced_http_handler(
    core_config: None,
) -> None:
    """HTTP builders use the same pre-construction reachability check."""
    with pytest.raises(ValueError, match=UNREFERENCED_DEFINITION_MESSAGE) as excinfo:
        (
            RuntimeConfigBuilder()
            .with_handler(
                "orphan",
                HTTPHandlerBuilder().with_endpoint("http://127.0.0.1:9/log"),
            )
            .apply()
        )

    assert "orphan" in str(excinfo.value), (
        "the unreferenced HTTP handler error must identify its configuration id"
    )


def test_runtime_apply_of_empty_transaction_is_a_noop(core_config: None) -> None:
    """An empty transaction must succeed without disturbing the registries."""
    logger = get_logger("core")
    before = logger.handler_ptrs_for_test()

    RuntimeConfigBuilder().apply()

    assert logger.handler_ptrs_for_test() == before, (
        "an empty RuntimeConfigBuilder().apply() must be a successful no-op "
        "that leaves the existing attachment graph untouched"
    )
    assert logger.log("DEBUG", "still attached") is not None, (
        "the core logger must still reach its handlers after an empty apply()"
    )


def test_runtime_apply_defines_and_attaches_in_one_transaction(
    core_config: None,
) -> None:
    """A definition attached in the same transaction must be published and live."""
    logger = get_logger("core")
    before = logger.handler_ptrs_for_test()

    (
        RuntimeConfigBuilder()
        .with_handler("stdout", StreamHandlerBuilder.stdout())
        .with_logger("core", LoggerMutationBuilder().append_handlers(["stdout"]))
        .apply()
    )

    after = logger.handler_ptrs_for_test()
    assert after[: len(before)] == before, (
        "a definition attached in the same transaction must join the logger "
        f"without disturbing the existing handlers; went from {before} to {after}"
    )
    assert len(after) == len(before) + 1, (
        f"the appended handler must add exactly one handler; got {after}"
    )


def test_replacing_file_handler_id_keeps_one_live_handler(
    core_config: None, tmp_path: Path
) -> None:
    """Repeatedly replacing one reachable file handler must not accumulate handlers.

    Each round rebuilds the handler under the same id, and the superseded
    object must be dropped once no logger references it. Object release itself
    is pinned more precisely by the Rust lifetime tests; what this test adds is
    that the public API keeps the live attachment set bounded across churn.
    """
    logger = get_logger("core")
    baseline = len(logger.handler_ptrs_for_test())

    for round_index in range(CHURN_ROUNDS):
        replacement = tmp_path / f"replacement-{round_index}.log"
        (
            RuntimeConfigBuilder()
            .with_handler("log", FileHandlerBuilder(str(replacement)))
            .with_logger("core", LoggerMutationBuilder().append_handlers(["log"]))
            .apply()
        )
        live = logger.handler_ptrs_for_test()
        expected = baseline + 1
        assert len(live) == expected, (
            f"round {round_index}: appending the reachable id 'log' must settle "
            f"at {expected} live handlers, but found {len(live)}"
        )
        live_path = tmp_path / f"live-{round_index}.log"
        (
            RuntimeConfigBuilder()
            .with_handler("log", FileHandlerBuilder(str(live_path)))
            .apply()
        )
        assert len(logger.handler_ptrs_for_test()) == expected, (
            f"round {round_index}: replacing the reachable id 'log' must swap "
            "the handler in place rather than add another one"
        )

    assert logger.log("DEBUG", "after churn") is not None, (
        "the logger must still be usable once the replacement churn finishes"
    )
