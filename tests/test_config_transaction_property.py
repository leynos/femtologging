"""Property tests for failed `ConfigBuilder` reconfiguration transactions."""

from __future__ import annotations

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

import femtologging
from femtologging import (
    ConfigBuilder,
    LoggerConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
)
from femtologging._rust_compat import _runtime_attachment_state_for_test

type AttachmentState = tuple[tuple[str, ...], tuple[str, ...]] | None
type LoggerState = tuple[
    tuple[int, ...], tuple[int, ...], str, bool, bool, AttachmentState
]

_LOGGER_NAMES = ("api", "api.worker", "batch", "batch.worker")
_HANDLER_ID = "stderr"


def _attachment_state(name: str) -> AttachmentState:
    """Return immutable runtime attachment metadata for *name*."""
    state = _runtime_attachment_state_for_test(name)
    if state is None:
        return None
    handler_ids, filter_ids = state
    return tuple(handler_ids), tuple(filter_ids)


def _logger_state(name: str) -> LoggerState:
    """Capture identity, scalar settings, and attachment metadata for *name*."""
    logger = get_logger(name)
    return (
        tuple(logger.handler_ptrs_for_test()),
        tuple(logger.filter_ptrs_for_test()),
        logger.level,
        logger.propagate,
        logger.disabled,
        _attachment_state(name),
    )


def _with_invalid_logger(
    builder: ConfigBuilder, kind: str
) -> tuple[ConfigBuilder, type[Exception], str]:
    """Add an invalid logger scenario and return its expected error."""
    match kind:
        case "handler":
            logger = LoggerConfigBuilder().with_handlers(["missing"])
            return builder.with_logger("broken", logger), KeyError, "missing"
        case "filter":
            logger = LoggerConfigBuilder().with_filters(["missing"])
            return builder.with_logger("broken", logger), KeyError, "missing"
        case "name":
            # ConfigBuilder wraps the manager's invalid-name ValueError in its
            # public RuntimeError contract.
            return (
                builder.with_logger("broken..name", LoggerConfigBuilder()),
                RuntimeError,
                "consecutive dots",
            )
        case _:
            message = f"unknown invalid_reference: {kind!r}"
            raise ValueError(message)


# This stateful property resets global workers between examples. It checks
# atomicity, not elapsed time, so shared-host contention must not affect it.
@settings(max_examples=25, deadline=None)
@given(
    logger_order=st.lists(
        st.sampled_from(_LOGGER_NAMES),
        min_size=1,
        max_size=len(_LOGGER_NAMES),
        unique=True,
    ),
    attached_names=st.frozensets(st.sampled_from(_LOGGER_NAMES)),
    disable_existing_loggers=st.booleans(),
    invalid_reference=st.sampled_from(("handler", "filter", "name")),
)
def test_failed_reconfiguration_preserves_generated_logger_state(
    logger_order: list[str],
    attached_names: frozenset[str],
    *,
    disable_existing_loggers: bool,
    invalid_reference: str,
) -> None:
    """Invalid plans preserve generated logger attachments and metadata.

    The generated logger order covers dotted logger names and their ancestors.
    Each example resets the global manager itself because Hypothesis invokes the
    test body repeatedly inside pytest's single autouse-fixture lifetime. The
    extension crosses the Python/Rust boundary and acquires shared registry
    locks, so its wall-clock time varies with unrelated Cargo and test work;
    the property constrains state, not latency.
    """
    femtologging.reset_manager()
    try:
        initial = ConfigBuilder().with_handler(
            _HANDLER_ID, StreamHandlerBuilder.stderr()
        )
        initial = initial.with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        for name in logger_order:
            logger_config = LoggerConfigBuilder()
            if name in attached_names:
                logger_config = logger_config.with_handlers([_HANDLER_ID])
            initial = initial.with_logger(name, logger_config)
        initial.build_and_init()

        before = {name: _logger_state(name) for name in logger_order}
        failed = (
            ConfigBuilder()
            .with_root_logger(LoggerConfigBuilder().with_level("ERROR"))
            .with_disable_existing_loggers(disable_existing_loggers)
        )
        failed, expected_error, expected_match = _with_invalid_logger(
            failed, invalid_reference
        )

        with pytest.raises(expected_error, match=expected_match):
            failed.build_and_init()

        after = {name: _logger_state(name) for name in logger_order}
        assert after == before, (
            "a failed configuration must preserve each generated logger's "
            "handler and filter identity, level, propagation, disabled flag, "
            "and runtime attachment metadata"
        )
    finally:
        femtologging.reset_manager()
