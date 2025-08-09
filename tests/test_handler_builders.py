from __future__ import annotations

from pathlib import Path

import pytest
from pytest_bdd import given, scenarios, then, when, parsers

from femtologging import (
    FileHandlerBuilder,
    StreamHandlerBuilder,
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
def when_set_file_capacity(file_builder: FileHandlerBuilder, capacity: int) -> None:
    file_builder.with_capacity(capacity)


@when(parsers.parse("I set stream capacity {capacity:d}"))
def when_set_stream_capacity(
    stream_builder: StreamHandlerBuilder, capacity: int
) -> None:
    stream_builder.with_capacity(capacity)


@when("I set flush interval 2")
def when_set_flush_interval(file_builder: FileHandlerBuilder) -> None:
    file_builder.with_flush_interval(2)


@then("the file handler builder matches snapshot")
def then_file_builder_snapshot(file_builder: FileHandlerBuilder, snapshot) -> None:
    data = file_builder.as_dict()
    data["path"] = Path(data["path"]).name
    assert data == snapshot
    handler = file_builder.build()
    handler.close()


@then("building the file handler fails")
def then_file_builder_fails(file_builder: FileHandlerBuilder) -> None:
    with pytest.raises(ValueError):
        file_builder.build()


@then("the stream handler builder matches snapshot")
def then_stream_builder_snapshot(
    stream_builder: StreamHandlerBuilder, snapshot
) -> None:
    assert stream_builder.as_dict() == snapshot
    handler = stream_builder.build()
    handler.flush()


@then("building the stream handler fails")
def then_stream_builder_fails(stream_builder: StreamHandlerBuilder) -> None:
    with pytest.raises(ValueError):
        stream_builder.build()
