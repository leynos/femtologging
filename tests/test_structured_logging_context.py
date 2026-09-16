"""Integration tests for Python structured logging fields and scoped context."""

from __future__ import annotations

import asyncio
import time
import typing as typ
from contextlib import ExitStack
from decimal import Decimal

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from femtologging import (
    FemtoLogger,
    StreamHandlerBuilder,
    basicConfig,
    get_logger,
    log_context,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc


class RecordCollector:
    """Collect structured records delivered through ``handle_record``."""

    def __init__(self) -> None:
        """Initialise the empty collection of received records."""
        self.records: list[dict[str, object]] = []

    def handle(self, logger: str, level: str, message: str) -> None:
        """Provide the legacy handler interface required by ``add_handler``."""
        _ = (self.records, logger, level, message)

    def handle_record(self, record: dict[str, object]) -> None:
        """Store a record snapshot emitted by the logger worker."""
        self.records.append(dict(record))

    def flush(self) -> bool:
        """Acknowledge the flush protocol used by ``FemtoLogger``."""
        _ = self.records
        return True


def emitted_key_values(
    logger: FemtoLogger, collector: RecordCollector
) -> dict[str, str]:
    """Return the structured fields on *logger*'s latest captured record."""
    _ = logger
    for _ in range(20):
        if collector.records:
            break
        time.sleep(0.01)
    assert collector.records, "expected at least one captured record"
    metadata = collector.records[-1].get("metadata")
    assert isinstance(metadata, dict), f"unexpected metadata payload: {metadata!r}"
    key_values = metadata.get("key_values")
    assert isinstance(key_values, dict), (
        f"unexpected key_values payload: {key_values!r}"
    )
    return typ.cast("dict[str, str]", key_values)


def assert_invalid_info_extra_rejected(
    logger_name: str, extra: dict[str, str | int | float | bool | None]
) -> None:
    """Assert that ``logger.info`` rejects invalid ``extra`` before level filtering."""
    logger = FemtoLogger(logger_name)
    logger.set_level("ERROR")

    with pytest.raises(TypeError, match="context values must be"):
        logger.info("invalid", extra=extra)


def test_direct_logger_info_merges_scoped_log_context() -> None:
    """``logger.info`` should include scoped ``log_context`` metadata."""
    logger = FemtoLogger("ctx.direct")
    logger.set_level("INFO")
    collector = RecordCollector()
    logger.add_handler(collector)

    with log_context(request_id="abc123", user="alice"):
        output = logger.info("inside context")

    assert output is not None, "info() should emit at INFO level"
    assert emitted_key_values(logger, collector) == {
        "request_id": "abc123",
        "user": "alice",
    }, "scoped context fields were not attached to logger.info()"


def test_get_logger_info_preserves_context_at_root_handler() -> None:
    """``get_logger`` records retain context through root-handler propagation."""
    records: list[dict[str, object]] = []

    def capture(record: dict[str, object]) -> str:
        records.append(record)
        return "captured"

    handler = StreamHandlerBuilder.stderr().with_formatter(capture).build()
    basicConfig(level="INFO", force=True, handlers=[handler])
    logger = get_logger("probe")

    logger.info("outside")
    with log_context(correlation_id="abc123"):
        logger.info("inside")
    assert logger.flush_handlers(), "child logger worker did not flush"
    for _ in range(20):
        if len(records) == 2:
            break
        time.sleep(0.01)
    assert len(records) == 2, f"expected two root-handler records, got {records!r}"

    key_values = [
        typ.cast("dict[str, object]", record["metadata"])["key_values"]
        for record in records
    ]
    assert key_values == [{}, {"correlation_id": "abc123"}], (
        f"unexpected root-handler key-values: {key_values!r}"
    )


def test_logger_log_extra_merges_and_overrides_scoped_context() -> None:
    """``log`` should merge ``extra`` fields and override scoped collisions."""
    logger = FemtoLogger("ctx.log")
    logger.set_level("INFO")
    collector = RecordCollector()
    logger.add_handler(collector)

    with log_context(request_id="outer", user="alice"):
        output = logger.log(
            "INFO",
            "inside context",
            extra={"request_id": "inline", "attempt": 2},
        )

    assert output is not None, "log() should emit at INFO level"
    assert emitted_key_values(logger, collector) == {
        "attempt": "2",
        "request_id": "inline",
        "user": "alice",
    }, "inline fields did not override or merge with scoped context"


@pytest.mark.parametrize(
    "method_name", ["debug", "info", "warning", "error", "critical"]
)
def test_convenience_methods_accept_extra(method_name: str) -> None:
    """Every generated convenience method should forward ``extra`` fields."""
    logger = FemtoLogger(f"ctx.{method_name}")
    logger.set_level("DEBUG")
    collector = RecordCollector()
    logger.add_handler(collector)
    emit = typ.cast("cabc.Callable[..., str | None]", getattr(logger, method_name))

    output = emit("structured", extra={"method": method_name})

    assert output is not None, f"{method_name}() should emit at DEBUG level"
    assert emitted_key_values(logger, collector) == {"method": method_name}, (
        f"{method_name}() did not forward extra fields"
    )


def test_logger_info_rejects_invalid_extra_value_when_disabled() -> None:
    """Invalid ``extra`` must fail before the level gate can drop the record."""
    assert_invalid_info_extra_rejected(
        "ctx.invalid-extra",
        typ.cast(
            "dict[str, str | int | float | bool | None]",
            {"request_id": {"nested": "value"}},
        ),
    )


class FloatConvertible:
    """Expose ``__float__`` without being a Python ``float`` instance."""

    def __float__(self) -> float:
        """Return a numeric value for protocol-based conversion callers."""
        return 1.0


class BoolConvertible:
    """Expose ``__bool__`` without being a Python ``bool`` instance."""

    def __bool__(self) -> bool:
        """Return a truth value for protocol-based conversion callers."""
        return True


@pytest.mark.parametrize(
    "invalid_value", [Decimal("1.5"), FloatConvertible(), BoolConvertible()]
)
def test_logger_extra_rejects_protocol_convertible_values(
    invalid_value: object,
) -> None:
    """Only documented scalar instances may appear in ``extra`` fields."""
    assert_invalid_info_extra_rejected(
        "ctx.invalid-scalar",
        typ.cast(
            "dict[str, str | int | float | bool | None]",
            {"value": invalid_value},
        ),
    )


def test_log_context_leaks_between_asyncio_tasks_on_one_thread() -> None:
    """Scoped context follows the thread, rather than asyncio task, until #433."""
    logger = FemtoLogger("ctx.asyncio-thread-local")
    logger.set_level("INFO")
    collector = RecordCollector()
    logger.add_handler(collector)

    async def interleave_contexts() -> None:
        """Force two request contexts to overlap on the event-loop thread."""
        first_entered = asyncio.Event()
        second_entered = asyncio.Event()
        first_emitted = asyncio.Event()

        async def emit_first_request() -> None:
            """Emit after the second task pushes its context frame."""
            with log_context(correlation_id="first"):
                first_entered.set()
                await second_entered.wait()
                logger.info("first request")
                first_emitted.set()

        async def emit_second_request() -> None:
            """Emit after the first task has popped the shared top frame."""
            await first_entered.wait()
            with log_context(correlation_id="second"):
                second_entered.set()
                await first_emitted.wait()
                logger.info("second request")

        await asyncio.gather(emit_first_request(), emit_second_request())

    try:
        asyncio.run(interleave_contexts())
        deadline = time.monotonic() + 2
        while len(collector.records) < 2 and time.monotonic() < deadline:
            time.sleep(0.01)
        observed_key_values = [
            typ.cast("dict[str, object]", record["metadata"])["key_values"]
            for record in collector.records
        ]
        assert observed_key_values == [
            {"correlation_id": "second"},
            {"correlation_id": "first"},
        ], "thread-local context did not expose the controlled task interference"
    finally:
        logger.remove_handler(collector)


@pytest.fixture(scope="module")
def fixture_property_logger() -> cabc.Iterator[FemtoLogger]:
    """Provide one isolated logger worker for generated merge examples."""
    logger = FemtoLogger("ctx.generated-merge")
    logger.set_level("INFO")
    try:
        yield logger
    finally:
        logger.clear_handlers()


_CONTEXT_KEYS = st.sampled_from(("request_id", "region", "tenant", "user"))
_CONTEXT_VALUES = st.text(alphabet="abcdefghijklmnopqrstuvwxyz0123456789", max_size=24)
_CONTEXT_MAP = st.dictionaries(
    keys=_CONTEXT_KEYS,
    values=_CONTEXT_VALUES,
    max_size=4,
)


@settings(max_examples=25, deadline=None)
@given(
    scoped_frames=st.lists(_CONTEXT_MAP, max_size=4),
    explicit_fields=_CONTEXT_MAP,
)
def test_generated_context_merge_matches_last_write_wins_model(
    fixture_property_logger: FemtoLogger,
    scoped_frames: list[dict[str, str]],
    explicit_fields: dict[str, str],
) -> None:
    """Scoped frames merge in order and inline fields override every collision."""
    collector = RecordCollector()
    fixture_property_logger.add_handler(collector)
    expected: dict[str, str] = {}
    try:
        with ExitStack() as context_stack:
            for frame in scoped_frames:
                context_stack.enter_context(log_context(**frame))
                expected.update(frame)
            expected.update(explicit_fields)
            fixture_property_logger.info("generated merge", extra=explicit_fields)

        assert emitted_key_values(fixture_property_logger, collector) == expected, (
            "generated scoped and inline fields did not follow last-write-wins"
        )
    finally:
        fixture_property_logger.flush_handlers()
        fixture_property_logger.remove_handler(collector)
