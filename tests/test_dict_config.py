"""Tests for handler construction and schema validation in ``dictConfig``.

Filter-section behaviour and validation live in
``tests/test_dict_config_filters.py``.
"""

from __future__ import annotations

import contextlib
import datetime as dt
import typing as typ

import pytest

from femtologging import (
    _clear_timed_rotation_test_times_for_test,
    _has_test_util,
    _set_timed_rotation_test_times_for_test,
    dictConfig,
    get_logger,
    reset_manager,
    LoggerConfigBuilder,
    ConfigBuilder,
    PythonCallbackFilterBuilder,
    FileHandlerBuilder,
    FormatterBuilder,
)
from tests.helpers import poll_file_for_text

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path


"""Tests for handler construction and schema validation in ``dictConfig``.
Filter-section behaviour and validation live in
``tests/test_dict_config_filters.py``.
"""
if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path


"""Tests for handler construction and schema validation in ``dictConfig``.
Filter-section behaviour and validation live in
``tests/test_dict_config_filters.py``.
"""
if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path
"""Tests for handler construction and schema validation in ``dictConfig``.
Filter-section behaviour and validation live in
``tests/test_dict_config_filters.py``.
"""
if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path


@contextlib.contextmanager
def _timed_rotation_test_clock(
    times: list[int],
) -> cabc.Generator[None, None, None]:
    """Context manager to set up and tear down timed rotation test clock."""
    _set_timed_rotation_test_times_for_test(times)
    try:
        yield
    finally:
        _clear_timed_rotation_test_times_for_test()


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


def _run_timed_dictconfig_rotation_test(
    tmp_path: Path,
    filename: str,
    handler_config: dict[str, object],
    rotated_suffix: str,
) -> None:
    reset_manager()
    path = tmp_path / filename
    # test_times: [0] = handler init time, [1] = first log (no advance, same as init),
    # [2] = second log at +2s triggers rotation
    test_times = [
        int(dt.datetime(2026, 3, 12, 0, 0, 0, tzinfo=dt.UTC).timestamp() * 1000),
        int(dt.datetime(2026, 3, 12, 0, 0, 0, tzinfo=dt.UTC).timestamp() * 1000),
        int(dt.datetime(2026, 3, 12, 0, 0, 2, tzinfo=dt.UTC).timestamp() * 1000),
    ]

    with _timed_rotation_test_clock(test_times):
        cfg = {
            "version": 1,
            "handlers": {
                "f": {
                    "class": "logging.handlers.TimedRotatingFileHandler",
                    **handler_config,
                }
            },
            "root": {"level": "INFO", "handlers": ["f"]},
        }
        dictConfig(cfg)
        logger = get_logger("root")
        logger.log("INFO", "first")
        logger.log("INFO", "second")
        poll_file_for_text(path, "second", timeout=1.0)
        rotated = path.with_name(f"{path.name}.{rotated_suffix}")
        poll_file_for_text(rotated, "first", timeout=1.0)


def _args_config_factory(path: Path) -> dict[str, object]:
    return {"args": [str(path), "S", 1, 1], "kwargs": {"utc": True}}


def _kwargs_config_factory(path: Path) -> dict[str, object]:
    return {
        "kwargs": {
            "filename": str(path),
            "when": "MIDNIGHT",
            "interval": 1,
            "backupCount": 1,
            "utc": True,
            "atTime": dt.time(0, 0, 0, 123456),
        }
    }


@pytest.mark.skipif(
    not _has_test_util,
    reason="requires Rust extension built with the 'test-util' feature",
)
@pytest.mark.parametrize(
    ("filename", "config_factory", "rotated_suffix"),
    [
        (
            "timed.log",
            _args_config_factory,
            "2026-03-12_00-00-00",
        ),
        (
            "timed_kwargs.log",
            _kwargs_config_factory,
            "2026-03-11",
        ),
    ],
    ids=["positional_args", "stdlib_kwargs"],
)
def test_dict_config_timed_rotating_handler(
    tmp_path: Path,
    filename: str,
    config_factory: cabc.Callable[[Path], dict[str, object]],
    rotated_suffix: str,
) -> None:
    """DictConfig should construct timed rotating handlers via various config styles."""
    path = tmp_path / filename

    handler_config = config_factory(path)

    _run_timed_dictconfig_rotation_test(
        tmp_path,
        filename,
        handler_config,
        rotated_suffix,
    )


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
        (
            {"filters": "context"},
            "handler filters must be a list or tuple of strings",
            TypeError,
        ),
    ],
    ids=[
        "args-bytes",
        "kwargs-bytes",
        "args-type",
        "kwargs-type",
        "filters-must-be-a-string-list",
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

def test_handler_filter_enriches_records_from_propagated_loggers(
    tmp_path: Path,
) -> None:
    """A root handler filter must run for every logger that propagates to it."""
    reset_manager()
    path = tmp_path / "handler-filter.log"
    observed_logger_names: list[str] = []

    def add_correlation_id(record: object) -> bool:
        record_attributes = vars(record)
        observed_logger_names.append(typ.cast("str", record_attributes["name"]))
        record_attributes["correlation_id"] = "REQ-99"
        return True

    handler = (
        FileHandlerBuilder(str(path))
        .with_flush_after_records(1)
        .with_filters(["context"])
        .with_formatter(lambda record: repr(record["metadata"]["key_values"]))
    )
    config = (
        ConfigBuilder()
        .with_version(1)
        .with_filter("context", PythonCallbackFilterBuilder(add_correlation_id))
        .with_handler("output", handler)
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["output"])
        )
    )

    try:
        config.build_and_init()
        get_logger("episodic.api.authorization").info("authorised")
        get_logger("episodic.worker").info("processed")
        poll_file_for_text(path, "'correlation_id': 'REQ-99'", timeout=1.0)
    finally:
        reset_manager()

    assert observed_logger_names == ["episodic.api.authorization", "episodic.worker"]
    assert path.read_text().count("'correlation_id': 'REQ-99'") == 2

def test_formatter_builder_renders_structured_callback_field(tmp_path: Path) -> None:
    """A formatter builder must render a handler filter's structured field."""
    reset_manager()
    path = tmp_path / "formatter-builder.log"

    def add_correlation_id(record: object) -> bool:
        vars(record)["correlation_id"] = "REQ-99"
        return True

    handler = (
        FileHandlerBuilder(str(path))
        .with_flush_after_records(1)
        .with_filters(["context"])
        .with_formatter(
            FormatterBuilder().with_format("%(correlation_id)s %(message)s")
        )
    )
    config = (
        ConfigBuilder()
        .with_version(1)
        .with_filter("context", PythonCallbackFilterBuilder(add_correlation_id))
        .with_handler("output", handler)
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["output"])
        )
    )

    try:
        config.build_and_init()
        get_logger("episodic.api.authorization").info("authorised")
        poll_file_for_text(path, "REQ-99 authorised", timeout=1.0)
    finally:
        reset_manager()

    assert path.read_text().strip() == "REQ-99 authorised"

def test_registered_formatter_renders_structured_callback_field(tmp_path: Path) -> None:
    """A formatter identifier must resolve through the configuration registry."""
    reset_manager()
    path = tmp_path / "formatter-registry.log"

    def add_correlation_id(record: object) -> bool:
        vars(record)["correlation_id"] = "REQ-99"
        return True

    handler = (
        FileHandlerBuilder(str(path))
        .with_flush_after_records(1)
        .with_filters(["context"])
        .with_formatter("context")
    )
    config = (
        ConfigBuilder()
        .with_version(1)
        .with_formatter(
            "context", FormatterBuilder().with_format("%(correlation_id)s %(message)s")
        )
        .with_filter("context", PythonCallbackFilterBuilder(add_correlation_id))
        .with_handler("output", handler)
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["output"])
        )
    )

    try:
        config.build_and_init()
        get_logger("episodic.api.authorization").info("authorised")
        poll_file_for_text(path, "REQ-99 authorised", timeout=1.0)
    finally:
        reset_manager()

    assert path.read_text().strip() == "REQ-99 authorised"

def test_dict_config_resolves_registered_formatter(tmp_path: Path) -> None:
    """DictConfig must attach formatter definitions to named handlers."""
    reset_manager()
    path = tmp_path / "dict-config-formatter.log"
    config = {
        "version": 1,
        "formatters": {"message": {"format": "%(message)s"}},
        "handlers": {
            "output": {
                "class": "femtologging.FileHandler",
                "args": [str(path)],
                "formatter": "message",
            }
        },
        "root": {"level": "INFO", "handlers": ["output"]},
    }

    try:
        dictConfig(config)
        get_logger("episodic.api.authorization").info("authorised")
        poll_file_for_text(path, "authorised", timeout=1.0)
    finally:
        reset_manager()

    assert path.read_text().strip() == "authorised"
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
            r"filter 'f' must contain a 'level', 'name', or '\(\)' key",
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
