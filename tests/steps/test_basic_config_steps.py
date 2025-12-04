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

FEATURES = Path(__file__).resolve().parents[1] / "features"

scenarios(str(FEATURES / "basic_config.feature"))


@given("the logging system is reset")
def reset_logging() -> None:
    reset_manager()


@given("root logger has a handler")
def root_has_handler() -> None:
    handler = FemtoStreamHandler.stderr()
    get_logger("root").add_handler(handler)


@when(
    parsers.re(
        r'I call basicConfig with level "(?P<level>[^"]+)"(?: and force (?P<force>true))?'
    )
)
def call_basic_config(level: str, force: str | None) -> None:
    basicConfig(level=level, force=force is not None)


@then(parsers.parse('logging "{msg}" at "{level}" from root matches snapshot'))
def log_matches_snapshot(msg: str, level: str, snapshot) -> None:
    logger = get_logger("root")
    if level.upper() == "DEBUG":
        assert logger.log(level, msg) is None
    else:
        assert logger.log(level, msg) == snapshot


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
    with closing(_make_handler(handler, tmp_path)) as h, pytest.raises(ValueError):
        basicConfig(handlers=[h], stream=sys.stdout)


@then(
    parsers.parse(
        'calling basicConfig with handler "{handler}" and filename "{filename}" fails'
    )
)
def basic_config_handler_filename_invalid(
    handler: str, filename: str, tmp_path: Path
) -> None:
    with closing(_make_handler(handler, tmp_path)) as h, pytest.raises(ValueError):
        basicConfig(handlers=[h], filename=str(tmp_path / filename))


def _make_handler(name: str, tmp_path: Path):
    if name == "stream_handler":
        return FemtoStreamHandler.stderr()
    if name == "file_handler":
        return FemtoFileHandler(str(tmp_path / "dummy.log"))
    msg = f"unknown handler {name}"
    raise ValueError(msg)
