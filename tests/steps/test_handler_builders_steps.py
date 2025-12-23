"""Behaviour-driven tests for handler builders (file, rotating, stream, socket)."""

from __future__ import annotations

import re
import typing as typ
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

import femtologging.config as config_module
from femtologging import (
    FileHandlerBuilder,
    HandlerConfigError,
    HTTPHandlerBuilder,
    OverflowPolicy,
    RotatingFileHandlerBuilder,
    SocketHandlerBuilder,
    StreamHandlerBuilder,
)

if typ.TYPE_CHECKING:
    from syrupy import SnapshotAssertion

FileBuilder = FileHandlerBuilder | RotatingFileHandlerBuilder

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "handler_builders.feature"))


def _normalise_builder_path(data: dict[str, object]) -> dict[str, object]:
    """Return builder dict with path normalised to basename for snapshots."""
    path_value = str(data["path"])
    data["path"] = Path(path_value).name
    return data


def _require_rotating_builder(builder: FileBuilder) -> RotatingFileHandlerBuilder:
    """Validate that a file builder targets rotation-specific operations."""
    if not isinstance(builder, RotatingFileHandlerBuilder):
        _fail_rotating_builder_requirement(builder)
    return builder


def _fail_rotating_builder_requirement(builder: FileBuilder) -> typ.NoReturn:
    """Raise a consistent failure for steps that assume a rotating builder."""
    msg = (
        "rotating builder step requires RotatingFileHandlerBuilder, "
        f"got {type(builder).__name__}"
    )
    raise AssertionError(msg)


def _build_flush_close(builder: HTTPHandlerBuilder) -> None:
    """Build, flush, and close a handler from a builder."""
    handler = builder.build()
    ok = handler.flush()
    assert ok, "handler.flush() timed out"
    handler.close()


@given('a FileHandlerBuilder for path "test.log"', target_fixture="file_builder")
def given_file_builder(tmp_path: Path) -> FileHandlerBuilder:
    path = tmp_path / "test.log"
    return FileHandlerBuilder(str(path))


@given(
    'a RotatingFileHandlerBuilder for path "test.log"', target_fixture="file_builder"
)
def given_rotating_file_builder(tmp_path: Path) -> RotatingFileHandlerBuilder:
    path = tmp_path / "test.log"
    return RotatingFileHandlerBuilder(str(path))


@given(
    'a dictConfig RotatingFileHandlerBuilder for path "test.log"',
    target_fixture="file_builder",
)
def given_dictconfig_rotating_file_builder(
    tmp_path: Path,
) -> RotatingFileHandlerBuilder:
    path = tmp_path / "test.log"
    # Use the dictConfig conversion helper to mirror Python logging schema
    # without mutating global configuration. The function is internal but
    # provides the only builder-returning path needed for this scenario.
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
    data = _normalise_builder_path(file_builder.as_dict())
    assert data == snapshot, "file builder dict must match snapshot"
    handler = file_builder.build()
    handler.close()


@then("the rotating file handler builder matches snapshot")
def then_rotating_file_builder_snapshot(
    file_builder: FileBuilder, snapshot: SnapshotAssertion
) -> None:
    rotating = _require_rotating_builder(file_builder)
    data = _normalise_builder_path(rotating.as_dict())
    assert data == snapshot, "rotating file builder dict must match snapshot"
    handler = rotating.build()
    handler.close()


@then("the file handler builder with timeout overflow matches snapshot")
def then_file_builder_timeout_snapshot(
    file_builder: FileHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    data = _normalise_builder_path(file_builder.as_dict())
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
    with pytest.raises(HandlerConfigError, match=re.escape(message)):
        socket_builder.build()


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


# --- HTTP Handler Steps ---


@given(
    parsers.parse('an HTTPHandlerBuilder for URL "{url}"'),
    target_fixture="http_builder",
)
def given_http_builder(url: str) -> HTTPHandlerBuilder:
    return HTTPHandlerBuilder().with_url(url)


@given("an empty HTTPHandlerBuilder", target_fixture="http_builder")
def given_empty_http_builder() -> HTTPHandlerBuilder:
    return HTTPHandlerBuilder()


@when("I set HTTP method POST", target_fixture="http_builder")
def when_set_http_method_post(http_builder: HTTPHandlerBuilder) -> HTTPHandlerBuilder:
    return http_builder.with_method("POST")


@when(
    parsers.parse("I set HTTP connect timeout {timeout:d}"),
    target_fixture="http_builder",
)
def when_set_http_connect_timeout(
    http_builder: HTTPHandlerBuilder, timeout: int
) -> HTTPHandlerBuilder:
    return http_builder.with_connect_timeout_ms(timeout)


@when(
    parsers.parse("I set HTTP write timeout {timeout:d}"),
    target_fixture="http_builder",
)
def when_set_http_write_timeout(
    http_builder: HTTPHandlerBuilder, timeout: int
) -> HTTPHandlerBuilder:
    return http_builder.with_write_timeout_ms(timeout)


@when("I enable JSON format", target_fixture="http_builder")
def when_enable_json_format(http_builder: HTTPHandlerBuilder) -> HTTPHandlerBuilder:
    return http_builder.with_json_format()


@when(
    parsers.parse('I set basic auth user "{user}" password "{password}"'),
    target_fixture="http_builder",
)
def when_set_basic_auth(
    http_builder: HTTPHandlerBuilder, user: str, password: str
) -> HTTPHandlerBuilder:
    return http_builder.with_basic_auth(user, password)


@when(parsers.parse('I set bearer token "{token}"'), target_fixture="http_builder")
def when_set_bearer_token(
    http_builder: HTTPHandlerBuilder, token: str
) -> HTTPHandlerBuilder:
    return http_builder.with_bearer_token(token)


@when(parsers.parse('I set record fields to "{fields}"'), target_fixture="http_builder")
def when_set_record_fields(
    http_builder: HTTPHandlerBuilder, fields: str
) -> HTTPHandlerBuilder:
    field_list = [f.strip() for f in fields.split(",")]
    return http_builder.with_record_fields(field_list)


@then("the HTTP handler builder matches snapshot")
def then_http_builder_snapshot(
    http_builder: HTTPHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    assert http_builder.as_dict() == snapshot, "HTTP builder dict must match snapshot"
    _build_flush_close(http_builder)


@then("the JSON HTTP handler builder matches snapshot")
def then_json_http_builder_snapshot(
    http_builder: HTTPHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    data = http_builder.as_dict()
    assert data.get("format") == "json", "must have JSON format"
    assert data == snapshot, "JSON HTTP builder dict must match snapshot"
    _build_flush_close(http_builder)


@then("the HTTP handler builder with auth matches snapshot")
def then_http_builder_auth_snapshot(
    http_builder: HTTPHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    data = http_builder.as_dict()
    assert data.get("auth_type") == "basic", "must have basic auth"
    assert data == snapshot, "HTTP builder with auth must match snapshot"
    _build_flush_close(http_builder)


@then("the HTTP handler builder with bearer matches snapshot")
def then_http_builder_bearer_snapshot(
    http_builder: HTTPHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    data = http_builder.as_dict()
    assert data.get("auth_type") == "bearer", "must have bearer auth"
    assert data == snapshot, "HTTP builder with bearer must match snapshot"
    _build_flush_close(http_builder)


@then("the HTTP handler builder with fields matches snapshot")
def then_http_builder_fields_snapshot(
    http_builder: HTTPHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    data = http_builder.as_dict()
    assert "record_fields" in data, "must have record_fields"
    assert data == snapshot, "HTTP builder with fields must match snapshot"
    _build_flush_close(http_builder)


@then(parsers.parse('building the HTTP handler fails with "{message}"'))
def then_http_builder_fails(http_builder: HTTPHandlerBuilder, message: str) -> None:
    with pytest.raises(HandlerConfigError, match=re.escape(message)):
        http_builder.build()
