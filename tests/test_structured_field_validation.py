"""Public Python-boundary validation tests for structured log fields."""

from __future__ import annotations

import collections.abc as cabc
import typing as typ
from decimal import Decimal

import pytest

from femtologging import FemtoLogger, log_context
from tests.test_structured_logging_context import RecordCollector, emitted_key_values

type StructuredScalar = str | int | float | bool | None


class CustomMapping(cabc.Mapping[str, StructuredScalar]):
    """Expose structured fields through the general Python mapping protocol."""

    def __init__(self, fields: dict[str, StructuredScalar]) -> None:
        """Store fields used by the mapping protocol methods."""
        self.fields = fields

    def __getitem__(self, key: str) -> StructuredScalar:
        """Return the field associated with *key*."""
        return self.fields[key]

    def __iter__(self) -> cabc.Iterator[str]:
        """Iterate over field names."""
        return iter(self.fields)

    def __len__(self) -> int:
        """Return the number of exposed fields."""
        return len(self.fields)


class MalformedItems:
    """Expose an invalid ``items()`` entry for conversion error coverage."""

    def items(self) -> list[tuple[str]]:
        """Return an item with no value component."""
        return [("key",)]


@pytest.mark.parametrize(
    ("scalar", "expected"),
    [
        ("text", "text"),
        (17, "17"),
        (1.5, "1.5"),
        (True, "True"),
        (None, "None"),
    ],
)
@pytest.mark.parametrize("route", ["extra", "log_context"])
def test_public_structured_fields_accept_documented_scalars(
    *, scalar: str | int | float | bool | None, expected: str, route: str
) -> None:
    """Both public field routes accept each documented scalar type."""
    logger = FemtoLogger(f"ctx.scalar.{route}")
    logger.set_level("INFO")
    collector = RecordCollector()
    logger.add_handler(collector)

    if route == "extra":
        output = logger.info("scalar", extra={"value": scalar})
    else:
        with log_context(value=scalar):
            output = logger.info("scalar")

    assert output is not None, "INFO-level scalar record should be emitted"
    assert emitted_key_values(logger, collector) == {"value": expected}, (
        f"{route} did not preserve the documented scalar value"
    )


def test_logger_info_accepts_a_custom_mapping_for_extra() -> None:
    """``extra`` accepts user mappings, not only built-in dictionaries."""
    logger = FemtoLogger("ctx.custom-mapping")
    logger.set_level("INFO")
    collector = RecordCollector()
    logger.add_handler(collector)

    output = logger.info("custom mapping", extra=CustomMapping({"value": 3}))

    assert output is not None, "INFO-level custom mapping record should be emitted"
    assert emitted_key_values(logger, collector) == {"value": "3"}, (
        "custom Mapping fields were not preserved"
    )


@pytest.mark.parametrize(
    ("extra", "message"),
    [
        ([], "context must be a mapping"),
        ({1: "value"}, "context keys must be strings"),
        (MalformedItems(), "context items must contain key-value pairs"),
        ({"value": []}, "context values must be"),
    ],
    ids=["non-mapping", "non-string-key", "malformed-item", "unsupported-value"],
)
def test_logger_info_rejects_invalid_mapping_shapes(
    extra: object, message: str
) -> None:
    """Malformed mappings and unsupported values fail at the Python boundary."""
    logger = FemtoLogger("ctx.invalid-mapping")
    logger.set_level("INFO")

    with pytest.raises(TypeError, match=message):
        logger.info(
            "invalid mapping",
            extra=typ.cast("cabc.Mapping[str, str | int | float | bool | None]", extra),
        )


@pytest.mark.parametrize(
    "extra",
    [
        pytest.param({"k" * 65: "value"}, id="overlong-key"),
        pytest.param({"key": "v" * 1025}, id="overlong-value"),
        pytest.param(
            {f"key-{index:02d}": "v" for index in range(65)},
            id="too-many-keys",
        ),
        pytest.param(
            {f"k{index:02d}": "v" * 1024 for index in range(16)},
            id="aggregate-size",
        ),
    ],
)
def test_logger_info_rejects_oversized_extra_before_level_filtering(
    extra: dict[str, StructuredScalar],
) -> None:
    """Over-limit fields fail before a disabled logger can retain a record."""
    logger = FemtoLogger("ctx.oversized-extra")
    logger.set_level("ERROR")
    collector = RecordCollector()
    logger.add_handler(collector)

    try:
        with pytest.raises(ValueError, match="context"):
            logger.info("oversized fields", extra=extra)
        assert not collector.records, "invalid fields must not enqueue a record"
    finally:
        logger.clear_handlers()


def test_log_context_rejects_an_unsupported_scalar() -> None:
    """Scoped fields reject values outside the documented scalar types."""
    with (
        pytest.raises(TypeError, match="context values must be"),
        log_context(value=Decimal("1.5")),
    ):
        pytest.fail("unsupported scoped scalar unexpectedly entered context")


RESERVED_EXTRA_KEYS = [
    # Logger identity.
    "name",
    "logger",
    # Severity.
    "levelname",
    "level",
    "levelno",
    # Message.
    "msg",
    "message",
    # Source information.
    "filename",
    "lineno",
    "module",
    "module_path",
    "line_number",
    # Optional built-in payloads and femtologging metadata.
    "exc_info",
    "stack_info",
    "threadName",
    "metadata",
    "key_values",
]

NEAR_MISS_KEYS = ["Name", "logger_name", "msg_id", "request_id", "Level", "MESSAGE"]

EMIT_METHODS = ("debug", "info", "warning", "error", "critical", "log")


def _emit_with_extra(
    logger: FemtoLogger, method: str, extra: dict[str, StructuredScalar]
) -> None:
    """Invoke one logging route with a reserved structured field."""
    if method == "log":
        logger.log("DEBUG", "reserved field", extra=extra)
    else:
        getattr(logger, method)("reserved field", extra=extra)


@pytest.mark.parametrize("key", RESERVED_EXTRA_KEYS)
@pytest.mark.parametrize("method", EMIT_METHODS)
@pytest.mark.parametrize("logger_level", ["ERROR", "CRITICAL"])
def test_reserved_extra_keys_raise_value_error_before_level_filtering(
    *, key: str, method: str, logger_level: str
) -> None:
    """Every logging route rejects reserved names even when its level is disabled."""
    logger = FemtoLogger(f"ctx.reserved.{method}.{logger_level}.{key}")
    logger.set_level(logger_level)

    with pytest.raises(ValueError, match="reserved"):
        _emit_with_extra(logger, method, {key: "value"})


@pytest.mark.parametrize("key", RESERVED_EXTRA_KEYS)
def test_log_context_rejects_reserved_keys(*, key: str) -> None:
    """Scoped context refuses a reserved name at entry, before any record exists."""
    with pytest.raises(ValueError, match="reserved"), log_context(**{key: "value"}):
        pass


def test_rejected_nested_log_context_leaves_outer_context_active() -> None:
    """A rejected nested context must not disturb the active outer frame."""
    logger = FemtoLogger("ctx.nested.rejected")
    logger.set_level("INFO")
    collector = RecordCollector()
    logger.add_handler(collector)

    with log_context(request_id="outer"):
        with pytest.raises(ValueError, match="reserved"), log_context(name="inner"):
            pass
        assert logger.info("after rejected nested context") is not None, (
            "the outer context should still admit records after the rejection"
        )

    assert emitted_key_values(logger, collector) == {"request_id": "outer"}, (
        "the rejected inner frame must not replace or drop the outer context"
    )


@pytest.mark.parametrize("route", ["extra", "log_context"])
@pytest.mark.parametrize("key", NEAR_MISS_KEYS)
def test_near_miss_names_are_accepted_and_preserved_as_strings(
    *, route: str, key: str
) -> None:
    """Case-sensitive near misses remain legitimate custom structured fields."""
    logger = FemtoLogger(f"ctx.near.{route}.{key}")
    logger.set_level("INFO")
    collector = RecordCollector()
    logger.add_handler(collector)

    if route == "extra":
        output = logger.info("near miss", extra={key: 7})
    else:
        with log_context(**{key: 7}):
            output = logger.info("near miss")

    assert output is not None, "INFO-level near-miss record should be emitted"
    key_values = emitted_key_values(logger, collector)
    assert key_values == {key: "7"}, "near-miss field should survive as a string"
    assert all(isinstance(value, str) for value in key_values.values()), (
        "structured values should be stored as strings"
    )
