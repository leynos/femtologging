"""Tests for femtologging.dictConfig integration and behaviour."""

from __future__ import annotations

import typing as typ

import pytest

from femtologging import (
    dictConfig,
    get_logger,
    reset_manager,
)
from tests.helpers import poll_file_for_text

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
    poll_file_for_text(path, "file", timeout=1.0)


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
            r"filter 'f' must contain a 'level' or 'name' key",
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
        "filter-missing-keys",
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


# -- Filter section tests --


def test_dict_config_level_filter_suppresses_records() -> None:
    """A level filter configured via dictConfig should suppress records."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {"lvl": {"level": "INFO"}},
        "loggers": {"app": {"filters": ["lvl"]}},
        "root": {"level": "DEBUG"},
    }
    dictConfig(cfg)
    logger = get_logger("app")
    assert logger.log("INFO", "allowed") is not None
    assert logger.log("ERROR", "suppressed") is None


def test_dict_config_name_filter_suppresses_records() -> None:
    """A name filter configured via dictConfig should suppress records."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {"ns": {"name": "myapp"}},
        "loggers": {"myapp": {"filters": ["ns"]}},
        "root": {"level": "DEBUG"},
    }
    dictConfig(cfg)
    matching = get_logger("myapp")
    assert matching.log("INFO", "pass") is not None

    non_matching = get_logger("other")
    # "other" has no filters attached, so it should still emit.
    assert non_matching.log("INFO", "also pass") is not None


def test_dict_config_multiple_filters_on_logger() -> None:
    """Multiple filters on one logger should all be applied."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {
            "lvl": {"level": "INFO"},
            "ns": {"name": "multi"},
        },
        "loggers": {"multi": {"filters": ["lvl", "ns"]}},
        "root": {"level": "DEBUG"},
    }
    dictConfig(cfg)
    logger = get_logger("multi")
    assert logger.log("INFO", "pass both") is not None
    assert logger.log("ERROR", "blocked by level") is None


def test_dict_config_root_logger_with_filters() -> None:
    """Filters should be attachable to the root logger."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {"lvl": {"level": "INFO"}},
        "root": {"level": "DEBUG", "filters": ["lvl"]},
    }
    dictConfig(cfg)
    root = get_logger("root")
    assert root.log("INFO", "allowed") is not None
    assert root.log("ERROR", "blocked") is None


def test_dict_config_filter_missing_filter_id_raises() -> None:
    """Referencing a non-existent filter ID should raise."""
    reset_manager()
    cfg = {
        "version": 1,
        "loggers": {"app": {"filters": ["nonexistent"]}},
        "root": {"level": "DEBUG"},
    }
    with pytest.raises(KeyError, match="nonexistent"):
        dictConfig(cfg)


@pytest.mark.parametrize(
    ("filter_cfg", "msg", "expected_exc"),
    [
        (
            {"level": 42},
            r"filter 'f' level must be a string",
            TypeError,
        ),
        (
            {"name": 42},
            r"filter 'f' name must be a string",
            TypeError,
        ),
        (
            {"level": "INFO", "extra": True},
            r"filter 'f' has unsupported keys",
            ValueError,
        ),
        (
            {"level": "INFO", "name": "ns"},
            r"filter 'f' must contain 'level' or 'name', not both",
            ValueError,
        ),
    ],
    ids=[
        "level-type",
        "name-type",
        "unsupported-keys",
        "both-level-and-name",
    ],
)
def test_dict_config_filter_validation_errors(
    filter_cfg: dict[str, object],
    msg: str,
    expected_exc: type[Exception],
) -> None:
    """Malformed filter configurations raise the expected exception."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {"f": filter_cfg},
        "root": {},
    }
    with pytest.raises(expected_exc, match=msg):
        dictConfig(cfg)


def test_dict_config_logger_filters_type_validation() -> None:
    """Logger filters must be a list or tuple of strings."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {"lvl": {"level": "INFO"}},
        "loggers": {"app": {"filters": "lvl"}},
        "root": {"level": "DEBUG"},
    }
    with pytest.raises(TypeError, match="logger filters must be a list"):
        dictConfig(cfg)


def test_dict_config_logger_filters_non_string_items() -> None:
    """Logger filters containing non-string items should raise."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {"lvl": {"level": "INFO"}},
        "loggers": {"app": {"filters": ["lvl", 123]}},
        "root": {"level": "DEBUG"},
    }
    with pytest.raises(TypeError, match="logger filters must be a list"):
        dictConfig(cfg)


def test_dict_config_root_filters_non_string_items() -> None:
    """Root logger filters containing non-string items should raise."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {"lvl": {"level": "INFO"}},
        "root": {"level": "DEBUG", "filters": [123]},
    }
    with pytest.raises(TypeError, match="logger filters must be a list"):
        dictConfig(cfg)


def test_dict_config_root_missing_filter_id_raises() -> None:
    """Root logger referencing a non-existent filter ID should raise."""
    reset_manager()
    cfg = {
        "version": 1,
        "root": {"level": "DEBUG", "filters": ["nonexistent"]},
    }
    with pytest.raises(KeyError, match="nonexistent"):
        dictConfig(cfg)


def test_dict_config_filter_value_not_a_mapping() -> None:
    """A filter whose config value is not a mapping should raise."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {"f": "not-a-mapping"},
        "root": {},
    }
    with pytest.raises(TypeError, match="filter config must be a mapping"):
        dictConfig(cfg)


def test_dict_config_empty_filters_section() -> None:
    """An empty filters section should not cause errors."""
    reset_manager()
    cfg = {
        "version": 1,
        "filters": {},
        "root": {"level": "DEBUG"},
    }
    dictConfig(cfg)
    root = get_logger("root")
    assert root.log("INFO", "emit") is not None
