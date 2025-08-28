"""Tests for femtologging.dictConfig integration and behaviour."""

from __future__ import annotations

from pathlib import Path
import time

import pytest
from pytest_bdd import given, parsers, scenarios, then, when

from femtologging import dictConfig, get_logger, reset_manager

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
def configure_with_handler_class(cls: str) -> ValueError:
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": cls}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(ValueError) as exc:
        dictConfig(cfg)
    return exc.value


@then("dictConfig raises ValueError")
def dict_config_raises_value_error(config_error: ValueError) -> None:
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
    contents = ""
    deadline = time.time() + 1.0
    while time.time() < deadline:
        if path.exists():
            contents = path.read_text()
            if "file" in contents:
                break
        time.sleep(0.01)
    else:
        pytest.fail("log file not written in time")
    assert "file" in contents


@pytest.mark.parametrize(
    ("handler_config", "expected_error"),
    [
        ({"args": b"bytes"}, "handler 'h' args must not be bytes or bytearray"),
        (
            {"kwargs": {"path": b"oops"}},
            "handler 'h' kwargs values must not be bytes or bytearray",
        ),
        ({"args": 1}, "handler 'h' args must be a sequence"),
        ({"kwargs": []}, "handler 'h' kwargs must be a mapping"),
        ({"filters": []}, "handler filters are not supported"),
    ],
    ids=[
        "args-bytes",
        "kwargs-bytes",
        "args-type",
        "kwargs-type",
        "filters-unsupported",
    ],
)
def test_dict_config_handler_validation_errors(
    handler_config: dict[str, object],
    expected_error: str,
) -> None:
    """Test various handler validation errors in dictConfig."""
    reset_manager()
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": "femtologging.StreamHandler", **handler_config}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(ValueError, match=expected_error):
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
    ("config", "msg"),
    [
        ({"version": 1}, r"root logger configuration is required"),
        ({"version": 2, "root": {}}, r"(unsupported|invalid).+version"),
        (
            {
                "version": 1,
                "handlers": {"h": {"class": "unknown"}},
                "root": {"handlers": ["h"]},
            },
            r"(unknown|unsupported).+handler class",
        ),
        ({"version": 1, "filters": {"f": {}}, "root": {}}, r"filters.+not supported"),
        (
            {
                "version": 1,
                "disable_existing_loggers": "yes",
                "root": {"handlers": []},
            },
            r"disable_existing_loggers must be a bool",
        ),
        (
            {"version": 1, "loggers": {1: {}}, "root": {"handlers": []}},
            r"loggers section key.+must be a string",
        ),
        (
            {
                "version": 1,
                "loggers": {"a": {"handlers": "h"}},
                "root": {"handlers": []},
            },
            r"logger handlers must be a list or tuple of strings",
        ),
        (
            {
                "version": 1,
                "loggers": {"a": {"propagate": "yes"}},
                "root": {"handlers": []},
            },
            r"logger propagate must be a bool",
        ),
        (
            {
                "version": 1,
                "formatters": {"f": {"format": 1}},
                "handlers": {
                    "h": {"class": "femtologging.StreamHandler", "formatter": "f"}
                },
                "root": {"handlers": ["h"]},
            },
            r"formatter 'format' must be a string",
        ),
        (
            {
                "version": 1,
                "handlers": {
                    "h": {"class": "femtologging.StreamHandler", "formatter": "x"}
                },
                "root": {"handlers": ["h"]},
            },
            r"unknown formatter id",
        ),
    ],
    ids=[
        "root-missing",
        "version-unsupported",
        "handler-class-unknown",
        "filters-unsupported",
        "disable-existing-loggers-type",
        "logger-id-type",
        "logger-handlers-type",
        "logger-propagate-type",
        "formatter-value-type",
        "formatter-id-unknown",
    ],
)
def test_dict_config_invalid_configs(config: dict[str, object], msg: str) -> None:
    """Invalid configurations raise ``ValueError``."""
    reset_manager()
    with pytest.raises(ValueError, match=msg):
        dictConfig(config)
