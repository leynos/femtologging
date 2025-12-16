"""Tests for femtologging.dictConfig integration and behaviour."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import (
    dictConfig,
    get_logger,
    reset_manager,
)
from tests.helpers import _poll_file_for_text

if typ.TYPE_CHECKING:
    from pathlib import Path


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
    contents = _poll_file_for_text(path, "file", timeout=1.0)
    assert "file" in contents


@pytest.mark.parametrize(
    ("handler_config", "expected_error", "expected_exc"),
    [
        (
            {"args": b"bytes"},
            "handler 'h' args must not be bytes or bytearray",
            TypeError,
        ),
        (
            {"kwargs": {"path": b"oops"}},
            "handler 'h' kwargs values must not be bytes or bytearray",
            TypeError,
        ),
        ({"args": 1}, "handler 'h' args must be a sequence", TypeError),
        ({"kwargs": []}, "handler 'h' kwargs must be a mapping", TypeError),
        ({"filters": []}, "handler filters are not supported", ValueError),
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
    expected_exc: type[Exception],
) -> None:
    """Test various handler validation errors in dictConfig."""
    reset_manager()
    cfg = {
        "version": 1,
        "handlers": {"h": {"class": "femtologging.StreamHandler", **handler_config}},
        "root": {"level": "INFO", "handlers": ["h"]},
    }
    with pytest.raises(expected_exc, match=expected_error):
        dictConfig(cfg)


def test_dict_config_logger_filters_presence() -> None:
    """Logger filter entries must be rejected."""
    reset_manager()
    cfg = {
        "version": 1,
        "loggers": {"a": {"filters": []}},
        "root": {"handlers": []},
    }
    with pytest.raises(ValueError, match="filters are not supported"):
        dictConfig(cfg)


@pytest.mark.parametrize(
    ("config", "msg", "expected_exc"),
    [
        ({"version": 1}, r"root logger configuration is required", ValueError),
        ({"version": 2, "root": {}}, r"(unsupported|invalid).+version", ValueError),
        (
            {
                "version": 1,
                "handlers": {
                    "h": {"class": "femtologging.StreamHandler", "formatter": "f"}
                },
                "root": {"level": "INFO", "handlers": ["h"]},
            },
            r"unknown formatter id",
            ValueError,
        ),
        (
            {"version": 1, "filters": {"f": {}}, "root": {}},
            r"filters.+not supported",
            ValueError,
        ),
        (
            {
                "version": 1,
                "disable_existing_loggers": "yes",
                "root": {"handlers": []},
            },
            r"disable_existing_loggers must be a bool",
            TypeError,
        ),
        (
            {"version": 1, "loggers": {1: {}}, "root": {"handlers": []}},
            r"loggers section key.+must be a string",
            TypeError,
        ),
        (
            {
                "version": 1,
                "loggers": {"a": {"handlers": "h"}},
                "root": {"handlers": []},
            },
            r"logger handlers must be a list or tuple of strings",
            TypeError,
        ),
        (
            {
                "version": 1,
                "loggers": {"a": {"propagate": "yes"}},
                "root": {"handlers": []},
            },
            r"logger propagate must be a bool",
            TypeError,
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
            TypeError,
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
            ValueError,
        ),
    ],
    ids=[
        "root-missing",
        "version-unsupported",
        "formatter-id-missing",
        "filters-unsupported",
        "disable-existing-loggers-type",
        "logger-id-type",
        "logger-handlers-type",
        "logger-propagate-type",
        "formatter-value-type",
        "formatter-id-unknown",
    ],
)
def test_dict_config_invalid_configs(
    config: dict[str, object], msg: str, expected_exc: type[Exception]
) -> None:
    """Invalid configurations raise the expected exception type."""
    reset_manager()
    with pytest.raises(expected_exc, match=msg):
        dictConfig(config)
