"""Behaviour-driven tests for FileHandlerBuilder and StreamHandlerBuilder.

Scenarios cover capacity and interval configuration, dictionary snapshots,
and build-time failures.
"""

from __future__ import annotations

from pathlib import Path

import pytest
from pytest_bdd import given, scenarios, then, when, parsers
from syrupy import SnapshotAssertion

from femtologging import (
    FileHandlerBuilder,
    StreamHandlerBuilder,
    HandlerConfigError,
)

scenarios("features/handler_builders.feature")


@given('a FileHandlerBuilder for path "test.log"', target_fixture="file_builder")
def given_file_builder(tmp_path) -> FileHandlerBuilder:
    path = tmp_path / "test.log"
    return FileHandlerBuilder(str(path))


@given("a StreamHandlerBuilder targeting stdout", target_fixture="stream_builder")
def given_stream_stdout() -> StreamHandlerBuilder:
    return StreamHandlerBuilder.stdout()


@given("a StreamHandlerBuilder targeting stderr", target_fixture="stream_builder")
def given_stream_stderr() -> StreamHandlerBuilder:
    return StreamHandlerBuilder.stderr()


@when(parsers.parse("I set file capacity {capacity:d}"))
def when_set_file_capacity(
    file_builder: FileHandlerBuilder, capacity: int
) -> FileHandlerBuilder:
    return file_builder.with_capacity(capacity)


@when(parsers.parse("I set stream capacity {capacity:d}"))
def when_set_stream_capacity(
    stream_builder: StreamHandlerBuilder, capacity: int
) -> StreamHandlerBuilder:
    return stream_builder.with_capacity(capacity)


@when(parsers.parse("I set stream flush timeout {timeout:d}"))
def when_set_stream_flush_timeout(
    stream_builder: StreamHandlerBuilder, timeout: int
) -> StreamHandlerBuilder:
    return stream_builder.with_flush_timeout_ms(timeout)


@when(parsers.parse("I set flush record interval {interval:d}"))
def when_set_flush_record_interval(
    file_builder: FileHandlerBuilder, interval: int
) -> FileHandlerBuilder:
    return file_builder.with_flush_record_interval(interval)


@when("I set overflow policy to timeout with 500ms")
def when_set_overflow_policy_timeout(
    file_builder: FileHandlerBuilder,
) -> FileHandlerBuilder:
    return file_builder.with_overflow_policy("timeout", timeout_ms=500)


@when(parsers.parse('I set file formatter "{formatter_id}"'))
def when_set_file_formatter(
    file_builder: FileHandlerBuilder, formatter_id: str
) -> FileHandlerBuilder:
    return file_builder.with_formatter(formatter_id)


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
    data["path"] = Path(data["path"]).name
    assert data == snapshot, "file builder dict must match snapshot"
    handler = file_builder.build()
    handler.close()


@then("the file handler builder with timeout overflow matches snapshot")
def then_file_builder_timeout_snapshot(
    file_builder: FileHandlerBuilder, snapshot: SnapshotAssertion
) -> None:
    data = file_builder.as_dict()
    data["path"] = Path(data["path"]).name
    assert data["overflow_policy"] == "timeout", "must record timeout policy"
    assert data["timeout_ms"] == 500, "must record configured timeout"
    assert data == snapshot, "snapshot must include timeout fields"
    handler = file_builder.build()
    handler.close()


@then("building the file handler fails")
def then_file_builder_fails(file_builder: FileHandlerBuilder) -> None:
    with pytest.raises(HandlerConfigError):
        file_builder.build()


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


@then("building the stream handler fails")
def then_stream_builder_fails(stream_builder: StreamHandlerBuilder) -> None:
    with pytest.raises(HandlerConfigError):
        stream_builder.build()


@then(parsers.parse("setting stream flush timeout {timeout:d} fails"))
def then_setting_stream_flush_timeout_fails(
    stream_builder: StreamHandlerBuilder, timeout: int
) -> None:
    with pytest.raises(OverflowError):
        stream_builder.with_flush_timeout_ms(timeout)


@pytest.mark.parametrize(
    "ctor", [StreamHandlerBuilder.stdout, StreamHandlerBuilder.stderr]
)
def test_stream_builder_negative_capacity(ctor) -> None:
    builder = ctor()
    with pytest.raises(OverflowError):
        builder.with_capacity(-1)
