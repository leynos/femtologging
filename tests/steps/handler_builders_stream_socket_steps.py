"""BDD steps for stream and socket handler builders.

Star-imported by ``tests/steps/test_handler_builders_steps.py``; see
``tests/steps/handler_builders_support.py`` for the rationale.
"""

from __future__ import annotations

import re
import typing as typ

import pytest
from pytest_bdd import given, parsers, then, when

from femtologging import (
    HandlerConfigError,
    SocketHandlerBuilder,
    StreamHandlerBuilder,
)
from tests.steps.handler_builders_support import build_flush_close

if typ.TYPE_CHECKING:
    from syrupy import SnapshotAssertion


@given("a StreamHandlerBuilder targeting stdout", target_fixture="stream_builder")
def given_stream_stdout() -> StreamHandlerBuilder:
    """Create a stream builder targeting standard output."""
    return StreamHandlerBuilder.stdout()


@given("a StreamHandlerBuilder targeting stderr", target_fixture="stream_builder")
def given_stream_stderr() -> StreamHandlerBuilder:
    """Create a stream builder targeting standard error."""
    return StreamHandlerBuilder.stderr()


@given(
    parsers.parse('a SocketHandlerBuilder for host "{host}" port {port:d}'),
    target_fixture="socket_builder",
)
def given_socket_builder(host: str, port: int) -> SocketHandlerBuilder:
    """Create a socket builder configured for the requested TCP endpoint."""
    return SocketHandlerBuilder().with_tcp(host, port)


@given("an empty SocketHandlerBuilder", target_fixture="socket_builder")
def given_empty_socket_builder() -> SocketHandlerBuilder:
    """Create a socket builder without endpoint settings."""
    return SocketHandlerBuilder()


@when(parsers.parse("I set stream capacity {capacity:d}"))
def when_set_stream_capacity(
    stream_builder: StreamHandlerBuilder, capacity: int
) -> StreamHandlerBuilder:
    """Set the stream builder's buffer capacity."""
    return stream_builder.with_capacity(capacity)


@when(parsers.parse("I set stream flush after ms {timeout:d}"))
def when_set_stream_flush_after_ms(
    stream_builder: StreamHandlerBuilder, timeout: int
) -> StreamHandlerBuilder:
    """Set the stream builder's flush interval in milliseconds."""
    return stream_builder.with_flush_after_ms(timeout)


@when(parsers.parse('I set stream formatter "{formatter_id}"'))
def when_set_stream_formatter(
    stream_builder: StreamHandlerBuilder, formatter_id: str
) -> StreamHandlerBuilder:
    """Set the formatter identifier on the stream builder."""
    return stream_builder.with_formatter(formatter_id)


@when(
    parsers.parse("I set socket capacity {capacity:d}"), target_fixture="socket_builder"
)
def when_set_socket_capacity(
    socket_builder: SocketHandlerBuilder, capacity: int
) -> SocketHandlerBuilder:
    """Set the socket builder's buffer capacity."""
    return socket_builder.with_capacity(capacity)


@when(
    parsers.parse("I set socket connect timeout {timeout:d}"),
    target_fixture="socket_builder",
)
def when_set_socket_connect_timeout(
    socket_builder: SocketHandlerBuilder, timeout: int
) -> SocketHandlerBuilder:
    """Set the socket connection timeout in milliseconds."""
    return socket_builder.with_connect_timeout_ms(timeout)


@when(
    parsers.parse("I set socket write timeout {timeout:d}"),
    target_fixture="socket_builder",
)
def when_set_socket_write_timeout(
    socket_builder: SocketHandlerBuilder, timeout: int
) -> SocketHandlerBuilder:
    """Set the socket write timeout in milliseconds."""
    return socket_builder.with_write_timeout_ms(timeout)


@when(
    parsers.parse("I set socket max frame size {size:d}"),
    target_fixture="socket_builder",
)
def when_set_socket_max_frame(
    socket_builder: SocketHandlerBuilder, size: int
) -> SocketHandlerBuilder:
    """Set the maximum socket frame size."""
    return socket_builder.with_max_frame_size(size)


@when(
    parsers.parse('I set socket tls domain "{domain}"'),
    target_fixture="socket_builder",
)
def when_set_socket_tls_domain(
    socket_builder: SocketHandlerBuilder, domain: str
) -> SocketHandlerBuilder:
    """Configure TLS for the socket builder with certificate verification."""
    return socket_builder.with_tls(domain, insecure=False)


@then("the stream handler builder matches snapshot")
def then_stream_builder_snapshot(
    stream_builder: StreamHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    """Compare the stream builder mapping with its syrupy snapshot."""
    assert stream_builder.as_dict() == snapshot, (
        "stream builder dict must match snapshot"
    )
    build_flush_close(stream_builder)


@then("the socket handler builder matches snapshot")
def then_socket_builder_snapshot(
    socket_builder: SocketHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    """Compare the socket builder mapping with its syrupy snapshot."""
    assert socket_builder.as_dict() == snapshot, (
        "socket builder dict must match snapshot"
    )
    build_flush_close(socket_builder)


@then("building the stream handler fails")
def then_stream_builder_fails(stream_builder: StreamHandlerBuilder) -> None:
    """Verify that building the stream handler raises a configuration error."""
    with pytest.raises(HandlerConfigError):
        stream_builder.build()


@then(parsers.parse('building the socket handler fails with "{message}"'))
def then_socket_builder_fails(
    socket_builder: SocketHandlerBuilder, message: str
) -> None:
    """Verify that the socket builder raises the expected configuration error."""
    with pytest.raises(HandlerConfigError, match=re.escape(message)):
        socket_builder.build()


@then(parsers.parse("setting stream flush after ms {timeout:d} fails"))
def then_setting_stream_flush_after_ms_fails(
    stream_builder: StreamHandlerBuilder, timeout: int
) -> None:
    """Verify that an invalid stream flush interval raises its expected exception."""
    exc = ValueError if timeout == 0 else OverflowError
    with pytest.raises(exc):
        stream_builder.with_flush_after_ms(timeout)
