from __future__ import annotations

import sys
from contextlib import closing
from pathlib import Path

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import (
    FemtoFileHandler,
    FemtoStreamHandler,
    basicConfig,
    get_logger,
    reset_manager,
)

scenarios("features/basic_config.feature")


@given("the logging system is reset")
def reset_logging() -> None:
    reset_manager()


@given("root logger has a handler")
def root_has_handler() -> None:
    handler = FemtoStreamHandler.stderr()
    get_logger("root").add_handler(handler)


@when(parsers.parse('I call basicConfig with level "{level}"'))
def call_basic_config(level: str) -> None:
    basicConfig(level=level)


@when(parsers.parse('I call basicConfig with level "{level}" and force true'))
def call_basic_config_force(level: str) -> None:
    basicConfig(level=level, force=True)


@then(parsers.parse('logging "{msg}" at "{level}" from root matches snapshot'))
def log_matches_snapshot(msg: str, level: str, snapshot) -> None:
    logger = get_logger("root")
    result = logger.log(level, msg)
    if level.upper() == "DEBUG":
        assert result is None
    else:
        assert result == snapshot


@then(parsers.parse("root logger has {count:d} handler"))
def root_handler_count(count: int) -> None:
    logger = get_logger("root")
    assert len(logger.handler_ptrs_for_test()) == count


@then(
    parsers.parse(
        'calling basicConfig with filename "{filename}" and stream stdout fails'
    )
)
def basic_config_invalid(filename: str, tmp_path: Path) -> None:
    with pytest.raises(ValueError):
        basicConfig(filename=str(tmp_path / filename), stream=sys.stdout)


@then(
    parsers.parse(
        'calling basicConfig with handler "{handler}" and stream stdout fails'
    )
)
def basic_config_handler_stream_invalid(handler: str, tmp_path: Path) -> None:
    with closing(_make_handler(handler, tmp_path)) as h:
        with pytest.raises(ValueError):
            basicConfig(handlers=[h], stream=sys.stdout)


@then(
    parsers.parse(
        'calling basicConfig with handler "{handler}" and filename "{filename}" fails'
    )
)
def basic_config_handler_filename_invalid(
    handler: str, filename: str, tmp_path: Path
) -> None:
    with closing(_make_handler(handler, tmp_path)) as h:
        with pytest.raises(ValueError):
            basicConfig(handlers=[h], filename=str(tmp_path / filename))


def _make_handler(name: str, tmp_path: Path):
    if name == "stream_handler":
        return FemtoStreamHandler.stderr()
    if name == "file_handler":
        return FemtoFileHandler(str(tmp_path / "dummy.log"))
    raise ValueError(f"unknown handler {name}")
