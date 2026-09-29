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


def test_log_context_rejects_an_unsupported_scalar() -> None:
    """Scoped fields reject values outside the documented scalar types."""
    with (
        pytest.raises(TypeError, match="context values must be"),
        log_context(value=Decimal("1.5")),
    ):
        pytest.fail("unsupported scoped scalar unexpectedly entered context")
