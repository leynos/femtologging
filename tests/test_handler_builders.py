"""Unit tests for handler builders (file, rotating, stream)."""

from __future__ import annotations

import sys
import typing as typ

import pytest

from femtologging import (
    FileHandlerBuilder,
    OverflowPolicy,
    RotatingFileHandlerBuilder,
    StreamHandlerBuilder,
)
from tests.helpers import poll_file_for_text

if typ.TYPE_CHECKING:
    from pathlib import Path


@pytest.mark.parametrize("max_bytes", [-1, -100, -999999])
def test_with_max_bytes_negative_raises(tmp_path: Path, max_bytes: int) -> None:
    """Negative max_bytes values must be rejected."""
    builder = RotatingFileHandlerBuilder(str(tmp_path / "test.log"))

    with pytest.raises(ValueError, match="max_bytes"):
        builder.with_max_bytes(max_bytes)


@pytest.mark.parametrize("backup_count", [-1, -5, -1000])
def test_with_backup_count_negative_raises(tmp_path: Path, backup_count: int) -> None:
    """Negative backup_count values must be rejected."""
    builder = RotatingFileHandlerBuilder(str(tmp_path / "test.log"))

    with pytest.raises(ValueError, match="backup"):
        builder.with_backup_count(backup_count)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_negative_capacity(
    ctor: typ.Callable[[], StreamHandlerBuilder],
) -> None:
    """Stream handler capacity must be non-negative."""
    builder = ctor()
    with pytest.raises(OverflowError):
        builder.with_capacity(-1)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_negative_flush_after_ms(
    ctor: typ.Callable[[], StreamHandlerBuilder],
) -> None:
    """Negative flush timeouts must raise."""
    builder = ctor()
    with pytest.raises(OverflowError):
        builder.with_flush_after_ms(-1)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_zero_flush_after_ms(
    ctor: typ.Callable[[], StreamHandlerBuilder],
) -> None:
    """Zero flush timeout is invalid."""
    builder = ctor()
    with pytest.raises(ValueError, match="flush_after_ms must be greater than zero"):
        builder.with_flush_after_ms(0)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_large_flush_after_ms(
    ctor: typ.Callable[[], StreamHandlerBuilder],
) -> None:
    """Very large flush timeouts should round-trip in as_dict."""
    builder = ctor().with_flush_after_ms(1_000_000_000)
    data = builder.as_dict()
    ctor_name = getattr(ctor, "__name__", repr(ctor))
    assert data["flush_after_ms"] == 1_000_000_000, (
        "Stream handler builder flush timeout mismatch: "
        f"ctor={ctor_name} builder={builder!r} "
        f"expected=1_000_000_000 actual={data['flush_after_ms']} "
        f"data={data}"
    )


def test_file_builder_negative_flush_after_records(tmp_path: Path) -> None:
    """Negative flush record intervals must be rejected."""
    builder = FileHandlerBuilder(str(tmp_path / "negative_flush_interval.log"))
    with pytest.raises(OverflowError):
        builder.with_flush_after_records(-1)


def test_file_builder_large_flush_after_records(tmp_path: Path) -> None:
    """Large flush intervals should be preserved in configuration."""
    builder = FileHandlerBuilder(str(tmp_path / "large_flush_interval.log"))
    builder = builder.with_flush_after_records(1_000_000_000)
    data = builder.as_dict()
    assert data["flush_after_records"] == 1_000_000_000, (
        "File handler builder flush interval mismatch: "
        f"builder={builder!r} expected=1_000_000_000 "
        f"actual={data['flush_after_records']} data={data}"
    )


def test_file_builder_zero_flush_after_records(tmp_path: Path) -> None:
    """Zero flush record intervals are invalid."""
    builder = FileHandlerBuilder(str(tmp_path / "zero_flush_interval.log"))
    with pytest.raises(
        ValueError, match="flush_after_records must be greater than zero"
    ):
        builder.with_flush_after_records(0)


def test_file_builder_flush_after_records_overflow(tmp_path: Path) -> None:
    """Values larger than u64 max must raise OverflowError."""
    too_large = sys.maxsize * 2 + 2
    builder = FileHandlerBuilder(str(tmp_path / "overflow_flush_interval.log"))
    with pytest.raises(OverflowError):
        builder.with_flush_after_records(too_large)


def test_file_builder_timeout_requires_explicit_timeout(tmp_path: Path) -> None:
    """Providing non-OverflowPolicy values raises ``TypeError``."""
    builder = FileHandlerBuilder(str(tmp_path / "builder_timeout_missing.log"))
    with pytest.raises(TypeError):
        builder.with_overflow_policy(
            typ.cast("OverflowPolicy", "timeout")  # intentional runtime type breach
        )


def test_file_builder_timeout_rejects_zero_timeout(tmp_path: Path) -> None:
    """Zero timeout values are rejected for timeout overflow policy."""
    builder = FileHandlerBuilder(str(tmp_path / "builder_timeout_zero.log"))
    with pytest.raises(ValueError, match="timeout must be greater than zero"):
        builder.with_overflow_policy(OverflowPolicy.timeout(0))


def test_file_builder_accepts_inline_timeout(tmp_path: Path) -> None:
    """Inline timeout syntax is accepted for builder configuration."""
    builder = FileHandlerBuilder(str(tmp_path / "builder_timeout_inline.log"))
    builder = builder.with_overflow_policy(OverflowPolicy.timeout(125))
    handler = builder.build()
    handler.close()


def test_stream_builder_accepts_callable_formatter() -> None:
    """Callable formatters should be accepted by stream builder."""
    builder = StreamHandlerBuilder.stderr().with_formatter(
        lambda record: f"callable:{record['message']}"
    )
    handler = builder.build()
    handler.close()


@pytest.mark.parametrize(
    ("builder_factory", "log_filename"),
    [
        pytest.param(
            lambda base: FileHandlerBuilder(str(base / "callable_formatter.log")),
            "callable_formatter.log",
            id="file",
        ),
        pytest.param(
            lambda base: RotatingFileHandlerBuilder(
                str(base / "callable_rotating.log")
            ),
            "callable_rotating.log",
            id="rotating",
        ),
    ],
)
def test_file_builders_accept_callable_formatter(
    tmp_path: Path,
    builder_factory: typ.Callable[
        [Path], FileHandlerBuilder | RotatingFileHandlerBuilder
    ],
    log_filename: str,
) -> None:
    """Callable formatter support should extend to file-based builders."""
    path = tmp_path / log_filename
    builder = builder_factory(tmp_path).with_formatter(
        lambda record: f"callable:{record['message']}"
    )
    handler = builder.build()
    handler.handle("logger", "INFO", "hello")
    handler.close()
    poll_file_for_text(path, "callable:hello", timeout=1.0)


def test_builder_formatter_error_chain(tmp_path: Path) -> None:
    """Errors when adapting Python formatters include both failure causes."""

    class NotFormatter:
        def __str__(self) -> str:  # pragma: no cover - invoked via PyO3
            msg = "no string representation available"
            raise TypeError(msg)

    builder = FileHandlerBuilder(str(tmp_path / "formatter_error_chain.log"))
    with pytest.raises(TypeError) as excinfo:
        builder.with_formatter(NotFormatter())

    # The error message now consolidates both failure causes inline
    error_message = str(excinfo.value)
    assert "invalid formatter" in error_message, (
        "formatter error must mention invalid formatter"
    )
    assert "expected a string identifier or callable" in error_message, (
        "formatter error must mention expected formatter types"
    )


class TestFlushApiConsistency:
    """Tests verifying consistent flush parameter types across handler builders.

    Issue #168: FileHandlerBuilder and StreamHandlerBuilder now use u64 for
    flush parameters, ensuring type consistency while preserving distinct
    semantics (record-count vs time-based).
    """

    @staticmethod
    def test_file_and_stream_builders_accept_same_large_value(tmp_path: Path) -> None:
        """Both builders accept the same large u64-compatible value."""
        # Exercise wide-range handling with a value that fits in u64
        large_value = 2**63 - 1

        # FileHandlerBuilder.with_flush_after_records accepts u64
        file_builder = FileHandlerBuilder(str(tmp_path / "large.log"))
        file_builder = file_builder.with_flush_after_records(large_value)
        data = file_builder.as_dict()
        assert data["flush_after_records"] == large_value, (
            f"expected flush_after_records {large_value} for FileHandlerBuilder, "
            f"got {data['flush_after_records']}"
        )

        # StreamHandlerBuilder.with_flush_after_ms accepts u64
        stream_builder = StreamHandlerBuilder.stderr()
        stream_builder = stream_builder.with_flush_after_ms(large_value)
        data = stream_builder.as_dict()
        assert data["flush_after_ms"] == large_value, (
            f"expected flush_after_ms {large_value} for StreamHandlerBuilder, "
            f"got {data['flush_after_ms']}"
        )

    @staticmethod
    def test_flush_parameter_error_message_format_consistency(tmp_path: Path) -> None:
        """Zero-value error messages follow the same pattern across builders."""
        file_builder = FileHandlerBuilder(str(tmp_path / "zero.log"))
        stream_builder = StreamHandlerBuilder.stderr()

        with pytest.raises(ValueError, match="must be greater than zero"):
            file_builder.with_flush_after_records(0)

        with pytest.raises(ValueError, match="must be greater than zero"):
            stream_builder.with_flush_after_ms(0)

    @staticmethod
    def test_rotating_builder_inherits_file_builder_flush_type(tmp_path: Path) -> None:
        """RotatingFileHandlerBuilder uses same u64 type as FileHandlerBuilder."""
        large_value = 2**62

        rotating_builder = RotatingFileHandlerBuilder(str(tmp_path / "rotating.log"))
        rotating_builder = rotating_builder.with_flush_after_records(large_value)
        data = rotating_builder.as_dict()
        assert data["flush_after_records"] == large_value, (
            f"expected flush_after_records {large_value} for "
            f"RotatingFileHandlerBuilder, got {data['flush_after_records']}"
        )

    @staticmethod
    def test_rotating_builder_zero_flush_after_records_rejected(tmp_path: Path) -> None:
        """RotatingFileHandlerBuilder rejects zero flush interval with ValueError."""
        rotating_builder = RotatingFileHandlerBuilder(str(tmp_path / "rotating.log"))
        with pytest.raises(ValueError, match="must be greater than zero"):
            rotating_builder.with_flush_after_records(0)

    @staticmethod
    def test_rotating_builder_negative_raises_overflow(tmp_path: Path) -> None:
        """Negative flush intervals raise OverflowError (PyO3 u64 extraction)."""
        builder = RotatingFileHandlerBuilder(str(tmp_path / "negative.log"))
        with pytest.raises(OverflowError):
            builder.with_flush_after_records(-1)

    @staticmethod
    @pytest.mark.parametrize("interval", [1, 100, 1_000_000, 2**30])
    def test_valid_interval_round_trips_in_config(
        tmp_path: Path, interval: int
    ) -> None:
        """Valid non-zero intervals are preserved through as_dict()."""
        builder = FileHandlerBuilder(str(tmp_path / f"interval_{interval}.log"))
        builder = builder.with_flush_after_records(interval)
        data = builder.as_dict()
        assert data["flush_after_records"] == interval, (
            f"expected flush_after_records {interval} for FileHandlerBuilder, "
            f"got {data['flush_after_records']}"
        )
