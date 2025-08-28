"""Tests for femtologging.dictConfig integration and behaviour."""

from __future__ import annotations

from pathlib import Path
import time

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import ConfigBuilder, dictConfig, get_logger, reset_manager
from femtologging.config import _process_formatters, _process_handlers

scenarios("features/dict_config.feature")


@given("the logging system is reset")
def reset_logging() -> None:
    reset_manager()


@when("I configure dictConfig with a stream handler")
def configure_dict_config() -> None:
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": "femtologging.StreamHandler"}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    dictConfig(cfg)


@then(parsers.parse('logging "{msg}" at "{level}" from root matches snapshot'))
def log_matches_snapshot(msg: str, level: str, snapshot) -> None:
    logger = get_logger("root")
    assert logger.log(level, msg) == snapshot


@then("calling dictConfig with incremental true raises ValueError")
def dict_config_incremental_fails() -> None:
    with pytest.raises(ValueError, match="incremental configuration is not supported"):
        dictConfig({"version": 1, "incremental": True, "root": {}})


@when(
    parsers.parse('I configure dictConfig with handler class "{cls}"'),
    target_fixture="config_error",
)
def configure_with_handler_class(cls: str) -> BaseException:
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": cls}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(ValueError) as exc:
        dictConfig(cfg)
    return exc.value


@then("dictConfig raises ValueError")
def dict_config_raises_value_error(config_error: BaseException) -> None:
    assert isinstance(config_error, ValueError)


def test_dict_config_file_handler_args_kwargs(tmp_path: Path) -> None:
    """Verify args and kwargs are evaluated for handler construction."""
    reset_manager()
    path = tmp_path / "out.log"
    cfg = {
        "version": 1,
        "handlers": {
            "f": {
                "class": "femtologging.FileHandler",
                "args": f"('{path}',)",
                "kwargs": "{}",
            }
        },
        "root": {"level": "INFO", "handlers": ["f"]},
    }
    dictConfig(cfg)
    logger = get_logger("root")
    logger.log("INFO", "file")
    time.sleep(0.05)
    assert path.exists()
    contents = path.read_text()
    assert "file" in contents


def test_dict_config_args_reject_bytes() -> None:
    reset_manager()
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": "femtologging.StreamHandler", "args": b"bytes"}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(
        ValueError, match="handler 'h' args must not be bytes or bytearray"
    ):
        dictConfig(cfg)


def test_dict_config_kwargs_reject_bytes_value() -> None:
    reset_manager()
    cfg = {
        "version": 1,
        "handlers": {
            "h": {
                "class": "femtologging.StreamHandler",
                "kwargs": {"path": b"oops"},
            }
        },
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(
        ValueError,
        match="handler 'h' kwargs values must not be bytes or bytearray",
    ):
        dictConfig(cfg)


def test_process_formatters_apply_to_handlers() -> None:
    builder = ConfigBuilder().with_version(1)
    cfg = {
        "formatters": {"f": {"format": "%(message)s", "datefmt": "%H:%M"}},
        "handlers": {
            "h": {
                "class": "femtologging.StreamHandler",
                "formatter": "f",
            }
        },
    }
    _process_formatters(builder, cfg)
    _process_handlers(builder, cfg)
    state = builder.as_dict()
    fmt = state["formatters"]["f"]
    assert fmt["format"] == "%(message)s"
    assert fmt["datefmt"] == "%H:%M"
    assert state["handlers"]["h"]["formatter_id"] == "f"


def test_dict_config_handler_filters_presence() -> None:
    reset_manager()
    cfg = {
        "version": 1,
        "handlers": {
            "h": {
                "class": "femtologging.StreamHandler",
                "filters": [],
            }
        },
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(ValueError, match="handler filters are not supported"):
        dictConfig(cfg)


def test_dict_config_logger_filters_presence() -> None:
    reset_manager()
    cfg = {
        "version": 1,
        "loggers": {"a": {"filters": []}},
        "root": {"handlers": []},
    }
    with pytest.raises(ValueError, match="filters are not supported"):
        dictConfig(cfg)


@pytest.mark.parametrize(
    "config",
    [
        {"version": 2, "root": {}},
        {
            "version": 1,
            "handlers": {"h": {"class": "unknown"}},
            "root": {"handlers": ["h"]},
        },
        {"version": 1, "filters": {"f": {}}, "root": {}},
    ],
    ids=["version-unsupported", "handler-class-unknown", "filters-unsupported"],
)
def test_dict_config_invalid_configs(config: dict[str, object]) -> None:
    """Invalid configurations raise ``ValueError``."""
    reset_manager()
    with pytest.raises(ValueError):
        dictConfig(config)
