"""Integration tests for Python structured logging fields and scoped context."""

from __future__ import annotations

import time
import typing as typ
from decimal import Decimal

import numpy as np
import pytest

from femtologging import (
    FemtoLogger,
    StreamHandlerBuilder,
    basicConfig,
    get_logger,
    log_context,
)


class RecordCollector:
    """Collect structured records delivered through ``handle_record``."""

    def __init__(self) -> None:
        """Initialise the empty collection of received records."""
        self.records: list[dict[str, object]] = []

    def handle(self, logger: str, level: str, message: str) -> None:
        """Provide the legacy handler interface required by ``add_handler``."""
        _ = (self.records, logger, level, message)

    def handle_record(self, record: dict[str, object]) -> None:
        """Store the record emitted by the logger worker."""
        self.records.append(record)

    def flush(self) -> bool:
        """Acknowledge the flush protocol used by ``FemtoLogger``."""
        _ = self.records
        return True


def emitted_key_values(
    logger: FemtoLogger, collector: RecordCollector
) -> dict[str, str]:
    """Flush *logger* and return the structured fields on its latest record."""
    for _ in range(20):
        if collector.records:
            break
        logger.flush_handlers()
        time.sleep(0.01)
    assert collector.records, "expected at least one captured record"
    metadata = collector.records[-1].get("metadata")
    assert isinstance(metadata, dict), f"unexpected metadata payload: {metadata!r}"
    key_values = metadata.get("key_values")
    assert isinstance(key_values, dict), (
        f"unexpected key_values payload: {key_values!r}"
    )
    return typ.cast("dict[str, str]", key_values)


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
    }


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
    }


@pytest.mark.parametrize(
    "method_name", ["debug", "info", "warning", "error", "critical"]
)
def test_convenience_methods_accept_extra(method_name: str) -> None:
    """Every generated convenience method should forward ``extra`` fields."""
    logger = FemtoLogger(f"ctx.{method_name}")
    logger.set_level("DEBUG")
    collector = RecordCollector()
    logger.add_handler(collector)
    emit = typ.cast("typ.Callable[..., str | None]", getattr(logger, method_name))

    output = emit("structured", extra={"method": method_name})

    assert output is not None, f"{method_name}() should emit at DEBUG level"
    assert emitted_key_values(logger, collector) == {"method": method_name}


def test_logger_info_rejects_invalid_extra_value_when_disabled() -> None:
    """Invalid ``extra`` must fail before the level gate can drop the record."""
    logger = FemtoLogger("ctx.invalid-extra")
    logger.set_level("ERROR")

    with pytest.raises(TypeError, match="context values must be"):
        logger.info(
            "invalid",
            extra=typ.cast(
                "dict[str, str | int | float | bool | None]",
                {"request_id": {"nested": "value"}},
            ),
        )


class FloatConvertible:
    """Expose ``__float__`` without being a Python ``float`` instance."""

    def __float__(self) -> float:
        """Return a numeric value for protocol-based conversion callers."""
        return 1.0


@pytest.mark.parametrize(
    "invalid_value", [Decimal("1.5"), FloatConvertible(), np.bool_("true")]
)
def test_logger_extra_rejects_protocol_convertible_values(
    invalid_value: object,
) -> None:
    """Only documented scalar instances may appear in ``extra`` fields."""
    logger = FemtoLogger("ctx.invalid-scalar")
    logger.set_level("ERROR")

    with pytest.raises(TypeError, match="context values must be"):
        logger.info(
            "invalid",
            extra=typ.cast(
                "dict[str, str | int | float | bool | None]",
                {"value": invalid_value},
            ),
        )
