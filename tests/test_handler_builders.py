"""Unit tests for handler builders (file, rotating, stream)."""

from __future__ import annotations

import datetime as dt
import typing as typ

import pytest

from femtologging import (
    FileHandlerBuilder,
    OverflowPolicy,
    RotatingFileHandlerBuilder,
    StreamHandlerBuilder,
    TimedRotatingFileHandlerBuilder,
)
from tests.helpers import poll_file_for_text

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

try:
    from hypothesis import given
    from hypothesis import strategies as st
except ImportError:  # pragma: no cover - only on interpreters lacking Hypothesis
    # Hypothesis has no CPython 3.15 distribution yet; tracked by
    # femtologging issue #385.
    _FLUSH_INTERVAL_PROPERTY = pytest.mark.skip(
        reason="Hypothesis is unavailable on this interpreter (issue #385)"
    )
else:
    # The Rust builders extract flush parameters as u64, so the whole u64
    # range must round-trip through ``as_dict``.
    _FLUSH_INTERVAL_PROPERTY = given(
        interval=st.integers(min_value=1, max_value=2**64 - 1)
    )

# The builders record configuration eagerly but only touch the filesystem at
# ``build()``, so configuration-only tests can use a fixed placeholder path.
_CONFIG_ONLY_LOG_PATH = "config_only.log"


def _assert_config_value(
    data: dict[str, object],
    key: str,
    expected: object,
    context: str,
) -> None:
    """Assert that a builder's ``as_dict`` records ``expected`` under ``key``."""
    assert data.get(key) == expected, (
        f"{context}: expected {key}={expected!r}, got {data.get(key)!r}; data={data}"
    )


def _configure_file_flush(value: int) -> dict[str, object]:
    """Set the file builder's flush interval and return its configuration."""
    builder = FileHandlerBuilder(_CONFIG_ONLY_LOG_PATH)
    return builder.with_flush_after_records(value).as_dict()


def _configure_rotating_flush(value: int) -> dict[str, object]:
    """Set the rotating builder's flush interval and return its configuration."""
    builder = RotatingFileHandlerBuilder(_CONFIG_ONLY_LOG_PATH)
    return builder.with_flush_after_records(value).as_dict()


def _configure_stream_flush(value: int) -> dict[str, object]:
    """Set the stream builder's flush timeout and return its configuration."""
    return StreamHandlerBuilder.stderr().with_flush_after_ms(value).as_dict()


# Issue #168: the file, rotating, and stream builders share a u64 flush
# parameter type and a common rejection contract, while keeping distinct
# semantics (record counts versus milliseconds). Driving all three through one
# table keeps that consistency under test.
_FLUSH_CASES: typ.Final = {
    "file": (_configure_file_flush, "flush_after_records"),
    "rotating": (_configure_rotating_flush, "flush_after_records"),
    "stream": (_configure_stream_flush, "flush_after_ms"),
}

flush_config_cases = pytest.mark.parametrize(
    ("configure_flush", "config_key"),
    [pytest.param(*case, id=name) for name, case in _FLUSH_CASES.items()],
)

flush_zero_cases = pytest.mark.parametrize(
    ("configure_flush", "zero_error"),
    [
        pytest.param(
            configure_flush,
            f"{config_key} must be greater than zero",
            id=name,
        )
        for name, (configure_flush, config_key) in _FLUSH_CASES.items()
    ],
)

flush_setter_cases = pytest.mark.parametrize(
    "configure_flush",
    [pytest.param(case[0], id=name) for name, case in _FLUSH_CASES.items()],
)


@flush_zero_cases
def test_flush_setter_rejects_zero(
    configure_flush: cabc.Callable[[int], dict[str, object]],
    zero_error: str,
) -> None:
    """Zero flush intervals are invalid and name the offending parameter."""
    with pytest.raises(ValueError, match=zero_error):
        configure_flush(0)


@flush_setter_cases
@pytest.mark.parametrize("value", [-1, -5, -1_000_000])
def test_flush_setter_rejects_negative(
    configure_flush: cabc.Callable[[int], dict[str, object]],
    value: int,
) -> None:
    """Negative flush intervals overflow the builders' u64 extraction."""
    with pytest.raises(OverflowError):
        configure_flush(value)


@flush_config_cases
@pytest.mark.parametrize(
    "interval",
    [
        pytest.param(1, id="minimum"),
        pytest.param(100, id="small"),
        pytest.param(1_000_000, id="moderate"),
        pytest.param(1_000_000_000, id="large"),
        pytest.param(2**63 - 1, id="i64-max"),
        pytest.param(2**64 - 1, id="u64-max"),
    ],
)
def test_flush_interval_round_trips_named_examples(
    configure_flush: cabc.Callable[[int], dict[str, object]],
    config_key: str,
    interval: int,
) -> None:
    """Normative flush intervals survive a round trip through ``as_dict``."""
    _assert_config_value(
        configure_flush(interval),
        config_key,
        interval,
        f"flush interval round trip for {config_key}",
    )


@flush_config_cases
@_FLUSH_INTERVAL_PROPERTY
def test_flush_interval_round_trips_for_any_valid_value(
    configure_flush: cabc.Callable[[int], dict[str, object]],
    config_key: str,
    interval: int,
) -> None:
    """Every accepted flush interval is preserved verbatim in the config."""
    _assert_config_value(
        configure_flush(interval),
        config_key,
        interval,
        f"flush interval round trip for {config_key}",
    )


def test_flush_after_records_above_u64_max_overflows() -> None:
    """Values wider than u64 must raise ``OverflowError`` rather than wrap."""
    too_large = 2**64
    builder = FileHandlerBuilder(_CONFIG_ONLY_LOG_PATH)
    with pytest.raises(OverflowError):
        builder.with_flush_after_records(too_large)


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


def test_timed_builder_invalid_when_raises(tmp_path: Path) -> None:
    """Unsupported timed rotation values must be rejected."""
    builder = TimedRotatingFileHandlerBuilder(str(tmp_path / "timed.log"))
    with pytest.raises(ValueError, match="unsupported timed rotation value"):
        builder.with_when("fortnight")


def test_timed_builder_rejects_at_time_for_hourly(tmp_path: Path) -> None:
    """Hour-based timed rotation should reject at_time."""
    builder = TimedRotatingFileHandlerBuilder(str(tmp_path / "timed.log"))
    with pytest.raises(ValueError, match="at_time is only supported"):
        builder.with_at_time(dt.time(8, 15, 0))


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_negative_capacity(
    ctor: cabc.Callable[[], StreamHandlerBuilder],
) -> None:
    """Stream handler capacity must be non-negative."""
    builder = ctor()
    with pytest.raises(OverflowError):
        builder.with_capacity(-1)


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


def test_file_builder_records_inline_timeout(tmp_path: Path) -> None:
    """Inline timeout syntax is recorded in the builder configuration."""
    builder = FileHandlerBuilder(str(tmp_path / "builder_timeout_inline.log"))
    data = builder.with_overflow_policy(OverflowPolicy.timeout(125)).as_dict()

    _assert_config_value(data, "overflow_policy", "timeout", "inline timeout policy")
    _assert_config_value(data, "timeout_ms", 125, "inline timeout duration")


def test_stream_builder_applies_callable_formatter(
    capfd: pytest.CaptureFixture[str],
) -> None:
    """Callable formatters shape the text a stream handler emits."""
    handler = (
        StreamHandlerBuilder
        .stderr()
        .with_formatter(lambda record: f"callable:{record['message']}")
        .build()
    )
    try:
        handler.handle("logger", "INFO", "hello")
    finally:
        handler.close()

    captured = capfd.readouterr().err
    assert "callable:hello" in captured, (
        f"stream handler should apply the callable formatter; stderr={captured!r}"
    )


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
    builder_factory: cabc.Callable[
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
