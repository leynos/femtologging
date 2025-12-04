"""Unit tests for handler builders (file, rotating, stream, socket)."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import (
    FileHandlerBuilder,
    OverflowPolicy,
    RotatingFileHandlerBuilder,
    StreamHandlerBuilder,
)

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
def test_stream_builder_negative_flush_timeout(
    ctor: typ.Callable[[], StreamHandlerBuilder],
) -> None:
    """Negative flush timeouts must raise."""
    builder = ctor()
    with pytest.raises(OverflowError):
        builder.with_flush_timeout_ms(-1)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_zero_flush_timeout(
    ctor: typ.Callable[[], StreamHandlerBuilder],
) -> None:
    """Zero flush timeout is invalid."""
    builder = ctor()
    with pytest.raises(ValueError, match="flush_timeout_ms must be greater than zero"):
        builder.with_flush_timeout_ms(0)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_large_flush_timeout(
    ctor: typ.Callable[[], StreamHandlerBuilder],
) -> None:
    """Very large flush timeouts should round-trip in as_dict."""
    builder = ctor().with_flush_timeout_ms(1_000_000_000)
    data = builder.as_dict()
    ctor_name = getattr(ctor, "__name__", repr(ctor))
    assert data["flush_timeout_ms"] == 1_000_000_000, (
        "Stream handler builder flush timeout mismatch: "
        f"ctor={ctor_name} builder={builder!r} "
        f"expected=1_000_000_000 actual={data['flush_timeout_ms']} "
        f"data={data}"
    )


def test_file_builder_negative_flush_record_interval(tmp_path: Path) -> None:
    """Negative flush record intervals must be rejected."""
    builder = FileHandlerBuilder(str(tmp_path / "negative_flush_interval.log"))
    with pytest.raises(OverflowError):
        builder.with_flush_record_interval(-1)


def test_file_builder_large_flush_record_interval(tmp_path: Path) -> None:
    """Large flush intervals should be preserved in configuration."""
    builder = FileHandlerBuilder(str(tmp_path / "large_flush_interval.log"))
    builder = builder.with_flush_record_interval(1_000_000_000)
    data = builder.as_dict()
    assert data["flush_record_interval"] == 1_000_000_000, (
        "File handler builder flush interval mismatch: "
        f"builder={builder!r} expected=1_000_000_000 "
        f"actual={data['flush_record_interval']} data={data}"
    )


def test_file_builder_zero_flush_record_interval(tmp_path: Path) -> None:
    """Zero flush record intervals are invalid."""
    builder = FileHandlerBuilder(str(tmp_path / "zero_flush_interval.log"))
    with pytest.raises(
        ValueError, match="flush_record_interval must be greater than zero"
    ):
        builder.with_flush_record_interval(0)


def test_file_builder_timeout_requires_explicit_timeout(tmp_path: Path) -> None:
    """Providing non-OverflowPolicy values raises ``TypeError``."""
    builder = FileHandlerBuilder(str(tmp_path / "builder_timeout_missing.log"))
    with pytest.raises(TypeError):
        builder.with_overflow_policy("timeout")  # type: ignore[arg-type]


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


def test_file_builder_accepts_callable_formatter(tmp_path: Path) -> None:
    """Callable formatter support should extend to file builder."""
    path = tmp_path / "callable_formatter.log"
    builder = FileHandlerBuilder(str(path)).with_formatter(
        lambda record: f"callable:{record['message']}"
    )
    handler = builder.build()
    handler.handle("logger", "INFO", "hello")
    handler.close()
    contents = path.read_text()
    assert "callable:hello" in contents


def test_rotating_builder_accepts_callable_formatter(tmp_path: Path) -> None:
    """Callable formatter support should extend to rotating builder."""
    path = tmp_path / "callable_rotating.log"
    builder = RotatingFileHandlerBuilder(str(path)).with_formatter(
        lambda record: f"callable:{record['message']}"
    )
    handler = builder.build()
    handler.handle("logger", "INFO", "hello")
    handler.close()
    contents = path.read_text()
    assert "callable:hello" in contents


def test_builder_formatter_error_chain(tmp_path: Path) -> None:
    """Errors when adapting Python formatters preserve both failure causes."""

    class NotFormatter:
        def __str__(self) -> str:  # pragma: no cover - invoked via PyO3
            msg = "no string representation available"
            raise TypeError(msg)

    builder = FileHandlerBuilder(str(tmp_path / "formatter_error_chain.log"))
    with pytest.raises(TypeError) as excinfo:
        builder.with_formatter(NotFormatter())

    chain_messages: list[str] = [str(excinfo.value)]
    cause = excinfo.value.__cause__
    while cause is not None:
        chain_messages.append(str(cause))
        cause = cause.__cause__

    assert any(
        "formatter string identifier extraction failed" in message
        for message in chain_messages
    ), "string formatter failure should remain in the cause chain"
    assert any(
        "formatter must be callable or expose a format(record: Mapping) -> str method"
        in message
        for message in chain_messages
    ), "callable formatter failure should remain in the cause chain"
