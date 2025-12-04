"""Behaviour-driven tests for FileHandlerBuilder and StreamHandlerBuilder.

Scenarios cover capacity and interval configuration, dictionary snapshots,
and build-time failures.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import TYPE_CHECKING, NoReturn

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

import femtologging.config as config_module
from femtologging import (
    FileHandlerBuilder,
    HandlerConfigError,
    OverflowPolicy,
    RotatingFileHandlerBuilder,
    SocketHandlerBuilder,
    StreamHandlerBuilder,
)

if TYPE_CHECKING:
    from syrupy import SnapshotAssertion

type FileBuilder = FileHandlerBuilder | RotatingFileHandlerBuilder


def _require_rotating_builder(builder: FileBuilder) -> RotatingFileHandlerBuilder:
    """Validate that a file builder targets rotation-specific operations."""
    if isinstance(builder, RotatingFileHandlerBuilder):
        return builder
    _fail_rotating_builder_requirement(builder)
    return None


def _fail_rotating_builder_requirement(builder: FileBuilder) -> NoReturn:
    """Raise a consistent failure for steps that assume a rotating builder."""
    msg = (
        "rotating builder step requires RotatingFileHandlerBuilder, "
        f"got {type(builder).__name__}"
    )
    raise AssertionError(
        msg
    )


@pytest.mark.parametrize("max_bytes", [-1, -100, -999999])
def test_with_max_bytes_negative_raises(tmp_path, max_bytes: int) -> None:
    builder = RotatingFileHandlerBuilder(str(tmp_path / "test.log"))

    with pytest.raises(ValueError):
        builder.with_max_bytes(max_bytes)


@pytest.mark.parametrize("backup_count", [-1, -5, -1000])
def test_with_backup_count_negative_raises(tmp_path, backup_count: int) -> None:
    builder = RotatingFileHandlerBuilder(str(tmp_path / "test.log"))

    with pytest.raises(ValueError):
        builder.with_backup_count(backup_count)


scenarios("features/handler_builders.feature")


@given('a FileHandlerBuilder for path "test.log"', target_fixture="file_builder")
def given_file_builder(tmp_path) -> FileHandlerBuilder:
    path = tmp_path / "test.log"
    return FileHandlerBuilder(str(path))


@given(
    'a RotatingFileHandlerBuilder for path "test.log"', target_fixture="file_builder"
)
def given_rotating_file_builder(tmp_path) -> RotatingFileHandlerBuilder:
    path = tmp_path / "test.log"
    return RotatingFileHandlerBuilder(str(path))


@given(
    'a dictConfig RotatingFileHandlerBuilder for path "test.log"',
    target_fixture="file_builder",
)
def given_dictconfig_rotating_file_builder(tmp_path) -> RotatingFileHandlerBuilder:
    path = tmp_path / "test.log"
    builder = config_module._build_handler_from_dict(
        "h",
        {
            "class": "logging.handlers.RotatingFileHandler",
            "args": [str(path)],
        },
    )
    return _require_rotating_builder(builder)


@given("a StreamHandlerBuilder targeting stdout", target_fixture="stream_builder")
def given_stream_stdout() -> StreamHandlerBuilder:
    return StreamHandlerBuilder.stdout()


@given("a StreamHandlerBuilder targeting stderr", target_fixture="stream_builder")
def given_stream_stderr() -> StreamHandlerBuilder:
    return StreamHandlerBuilder.stderr()


@given(
    parsers.parse('a SocketHandlerBuilder for host "{host}" port {port:d}'),
    target_fixture="socket_builder",
)
def given_socket_builder(host: str, port: int) -> SocketHandlerBuilder:
    return SocketHandlerBuilder().with_tcp(host, port)


@given("an empty SocketHandlerBuilder", target_fixture="socket_builder")
def given_empty_socket_builder() -> SocketHandlerBuilder:
    return SocketHandlerBuilder()


@when(parsers.parse("I set file capacity {capacity:d}"))
def when_set_file_capacity(file_builder: FileBuilder, capacity: int) -> FileBuilder:
    return file_builder.with_capacity(capacity)


@when(parsers.parse("I set stream capacity {capacity:d}"))
def when_set_stream_capacity(
    stream_builder: StreamHandlerBuilder, capacity: int
) -> StreamHandlerBuilder:
    return stream_builder.with_capacity(capacity)


@when(
    parsers.parse("I set socket capacity {capacity:d}"), target_fixture="socket_builder"
)
def when_set_socket_capacity(
    socket_builder: SocketHandlerBuilder, capacity: int
) -> SocketHandlerBuilder:
    return socket_builder.with_capacity(capacity)


@when(parsers.parse("I set stream flush timeout {timeout:d}"))
def when_set_stream_flush_timeout(
    stream_builder: StreamHandlerBuilder, timeout: int
) -> StreamHandlerBuilder:
    return stream_builder.with_flush_timeout_ms(timeout)


@when(
    parsers.parse("I set socket connect timeout {timeout:d}"),
    target_fixture="socket_builder",
)
def when_set_socket_connect_timeout(
    socket_builder: SocketHandlerBuilder, timeout: int
) -> SocketHandlerBuilder:
    return socket_builder.with_connect_timeout_ms(timeout)


@when(
    parsers.parse("I set socket write timeout {timeout:d}"),
    target_fixture="socket_builder",
)
def when_set_socket_write_timeout(
    socket_builder: SocketHandlerBuilder, timeout: int
) -> SocketHandlerBuilder:
    return socket_builder.with_write_timeout_ms(timeout)


@when(
    parsers.parse("I set socket max frame size {size:d}"),
    target_fixture="socket_builder",
)
def when_set_socket_max_frame(
    socket_builder: SocketHandlerBuilder, size: int
) -> SocketHandlerBuilder:
    return socket_builder.with_max_frame_size(size)


@when(
    parsers.parse('I set socket tls domain "{domain}"'),
    target_fixture="socket_builder",
)
def when_set_socket_tls_domain(
    socket_builder: SocketHandlerBuilder, domain: str
) -> SocketHandlerBuilder:
    return socket_builder.with_tls(domain, insecure=False)


@when(parsers.parse("I set flush record interval {interval:d}"))
def when_set_flush_record_interval(
    file_builder: FileBuilder, interval: int
) -> FileBuilder:
    return file_builder.with_flush_record_interval(interval)


@when("I set overflow policy to timeout with 500ms")
def when_set_overflow_policy_timeout(
    file_builder: FileBuilder,
) -> FileBuilder:
    return file_builder.with_overflow_policy(OverflowPolicy.timeout(500))


@when(parsers.parse('I set file formatter "{formatter_id}"'))
def when_set_file_formatter(
    file_builder: FileBuilder, formatter_id: str
) -> FileBuilder:
    return file_builder.with_formatter(formatter_id)


@when(parsers.parse("I set max bytes {max_bytes:d}"))
def when_set_max_bytes(file_builder: FileBuilder, max_bytes: int) -> FileBuilder:
    rotating = _require_rotating_builder(file_builder)
    return rotating.with_max_bytes(max_bytes)


@when(parsers.parse("I set backup count {backup_count:d}"))
def when_set_backup_count(file_builder: FileBuilder, backup_count: int) -> FileBuilder:
    rotating = _require_rotating_builder(file_builder)
    return rotating.with_backup_count(backup_count)


@when(parsers.parse('I set stream formatter "{formatter_id}"'))
def when_set_stream_formatter(
    stream_builder: StreamHandlerBuilder, formatter_id: str
) -> StreamHandlerBuilder:
    return stream_builder.with_formatter(formatter_id)


@then("the file handler builder matches snapshot")
def then_file_builder_snapshot(
    file_builder: FileHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    data = file_builder.as_dict()
    path_value = str(data["path"])
    data["path"] = Path(path_value).name
    assert data == snapshot, "file builder dict must match snapshot"
    handler = file_builder.build()
    handler.close()


@then("the rotating file handler builder matches snapshot")
def then_rotating_file_builder_snapshot(
    file_builder: FileBuilder, snapshot: SnapshotAssertion
) -> None:
    rotating = _require_rotating_builder(file_builder)
    data = rotating.as_dict()
    path_value = str(data["path"])
    data["path"] = Path(path_value).name
    assert data == snapshot, "rotating file builder dict must match snapshot"
    handler = rotating.build()
    handler.close()


@then("the file handler builder with timeout overflow matches snapshot")
def then_file_builder_timeout_snapshot(
    file_builder: FileHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    data = file_builder.as_dict()
    path_value = str(data["path"])
    data["path"] = Path(path_value).name
    assert data["overflow_policy"] == "timeout", "must record timeout policy"
    assert data["timeout_ms"] == 500, "must record configured timeout"
    assert data == snapshot, "snapshot must include timeout fields"
    handler = file_builder.build()
    handler.close()


@then("building the file handler fails")
def then_file_builder_fails(file_builder: FileHandlerBuilder) -> None:
    with pytest.raises(HandlerConfigError):
        file_builder.build()


@then(parsers.parse('building the rotating file handler fails with "{message}"'))
def then_rotating_file_builder_fails(file_builder: FileBuilder, message: str) -> None:
    rotating = _require_rotating_builder(file_builder)
    with pytest.raises(HandlerConfigError, match=re.escape(message)):
        rotating.build()


@then(parsers.parse('setting max bytes {max_bytes:d} fails with "{message}"'))
def then_setting_max_bytes_fails(
    file_builder: FileBuilder, max_bytes: int, message: str
) -> None:
    rotating = _require_rotating_builder(file_builder)
    with pytest.raises(ValueError, match=re.escape(message)):
        rotating.with_max_bytes(max_bytes)


@then(parsers.parse('setting backup count {backup_count:d} fails with "{message}"'))
def then_setting_backup_count_fails(
    file_builder: FileBuilder, backup_count: int, message: str
) -> None:
    rotating = _require_rotating_builder(file_builder)
    with pytest.raises(ValueError, match=re.escape(message)):
        rotating.with_backup_count(backup_count)


@then(parsers.parse('setting zero rotation thresholds fails with "{message}"'))
def then_zero_rotation_thresholds_fail(file_builder: FileBuilder, message: str) -> None:
    rotating = _require_rotating_builder(file_builder)
    with pytest.raises(HandlerConfigError, match=re.escape(message)):
        rotating.with_max_bytes(0).with_backup_count(0).build()


@then("the stream handler builder matches snapshot")
def then_stream_builder_snapshot(
    stream_builder: StreamHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    assert stream_builder.as_dict() == snapshot, (
        "stream builder dict must match snapshot"
    )
    handler = stream_builder.build()
    handler.flush()
    handler.close()


@then("the socket handler builder matches snapshot")
def then_socket_builder_snapshot(
    socket_builder: SocketHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    assert socket_builder.as_dict() == snapshot, (
        "socket builder dict must match snapshot"
    )
    handler = socket_builder.build()
    handler.flush()
    handler.close()


@then("building the stream handler fails")
def then_stream_builder_fails(stream_builder: StreamHandlerBuilder) -> None:
    with pytest.raises(HandlerConfigError):
        stream_builder.build()


@then(parsers.parse('building the socket handler fails with "{message}"'))
def then_socket_builder_fails(
    socket_builder: SocketHandlerBuilder, message: str
) -> None:
    with pytest.raises(HandlerConfigError) as exc:
        socket_builder.build()
    assert message in str(exc.value)


@then(parsers.parse("setting stream flush timeout {timeout:d} fails"))
def then_setting_stream_flush_timeout_fails(
    stream_builder: StreamHandlerBuilder, timeout: int
) -> None:
    exc = ValueError if timeout == 0 else OverflowError
    with pytest.raises(exc):
        stream_builder.with_flush_timeout_ms(timeout)


@then(parsers.parse("setting flush record interval {interval:d} fails"))
def then_setting_flush_record_interval_fails(
    file_builder: FileBuilder, interval: int
) -> None:
    exc = ValueError if interval == 0 else OverflowError
    with pytest.raises(exc):
        file_builder.with_flush_record_interval(interval)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_negative_capacity(ctor) -> None:
    builder = ctor()
    with pytest.raises(OverflowError):
        builder.with_capacity(-1)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_negative_flush_timeout(ctor) -> None:
    builder = ctor()
    with pytest.raises(OverflowError):
        builder.with_flush_timeout_ms(-1)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_zero_flush_timeout(ctor) -> None:
    builder = ctor()
    with pytest.raises(ValueError):
        builder.with_flush_timeout_ms(0)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_large_flush_timeout(ctor) -> None:
    builder = ctor().with_flush_timeout_ms(1_000_000_000)
    data = builder.as_dict()
    assert data["flush_timeout_ms"] == 1_000_000_000, (
        "Stream handler builder flush timeout mismatch: "
        f"ctor={ctor.__name__} builder={builder!r} "
        f"expected=1_000_000_000 actual={data['flush_timeout_ms']} "
        f"data={data}"
    )


def test_file_builder_negative_flush_record_interval(tmp_path: Path) -> None:
    builder = FileHandlerBuilder(str(tmp_path / "negative_flush_interval.log"))
    with pytest.raises(OverflowError):
        builder.with_flush_record_interval(-1)


def test_file_builder_large_flush_record_interval(tmp_path: Path) -> None:
    builder = FileHandlerBuilder(str(tmp_path / "large_flush_interval.log"))
    builder = builder.with_flush_record_interval(1_000_000_000)
    data = builder.as_dict()
    assert data["flush_record_interval"] == 1_000_000_000, (
        "File handler builder flush interval mismatch: "
        f"builder={builder!r} expected=1_000_000_000 "
        f"actual={data['flush_record_interval']} data={data}"
    )


def test_file_builder_zero_flush_record_interval(tmp_path: Path) -> None:
    builder = FileHandlerBuilder(str(tmp_path / "zero_flush_interval.log"))
    with pytest.raises(ValueError):
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
    builder = StreamHandlerBuilder.stderr().with_formatter(
        lambda record: f"callable:{record['message']}"
    )
    handler = builder.build()
    handler.close()


def test_file_builder_accepts_callable_formatter(tmp_path: Path) -> None:
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
