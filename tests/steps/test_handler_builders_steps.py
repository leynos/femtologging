"""Behaviour-driven tests for FileHandlerBuilder and StreamHandlerBuilder."""

from __future__ import annotations

import re
from pathlib import Path
import typing as typ

import pytest
from pytest_bdd import given, parsers, scenarios, then, when
from syrupy import SnapshotAssertion

import femtologging.config as config_module
from femtologging import (
    FileHandlerBuilder,
    HandlerConfigError,
    OverflowPolicy,
    RotatingFileHandlerBuilder,
    SocketHandlerBuilder,
    StreamHandlerBuilder,
)

if typ.TYPE_CHECKING:
    type FileBuilder = FileHandlerBuilder | RotatingFileHandlerBuilder

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "handler_builders.feature"))


def _require_rotating_builder(builder: FileBuilder) -> RotatingFileHandlerBuilder:
    """Validate that a file builder targets rotation-specific operations."""
    if isinstance(builder, RotatingFileHandlerBuilder):
        return builder
    _fail_rotating_builder_requirement(builder)
    return None  # pragma: no cover - _fail_rotating_builder_requirement always raises


def _fail_rotating_builder_requirement(builder: FileBuilder) -> typ.NoReturn:
    """Raise a consistent failure for steps that assume a rotating builder."""
    msg = (
        "rotating builder step requires RotatingFileHandlerBuilder, "
        f"got {type(builder).__name__}"
    )
    raise AssertionError(msg)


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
def when_set_overflow_policy_timeout(file_builder: FileBuilder) -> FileBuilder:
    return file_builder.with_overflow_policy(OverflowPolicy.timeout(500))


@when(parsers.parse('I set file formatter "{formatter_id}"'))
def when_set_file_formatter(file_builder: FileBuilder, formatter_id: str) -> FileBuilder:
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
