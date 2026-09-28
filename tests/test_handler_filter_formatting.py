"""Integration tests for handler filters and structured formatters."""

from __future__ import annotations

import typing as typ

from femtologging import (
    ConfigBuilder,
    FileHandlerBuilder,
    FormatterBuilder,
    LoggerConfigBuilder,
    PythonCallbackFilterBuilder,
    dictConfig,
    get_logger,
    reset_manager,
)
from tests.helpers import poll_file_for_text

if typ.TYPE_CHECKING:
    from pathlib import Path


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

    assert observed_logger_names == ["episodic.api.authorization", "episodic.worker"], (
        "the handler filter must observe each propagated logger"
    )
    assert path.read_text().count("'correlation_id': 'REQ-99'") == 2, (
        "the handler formatter must render structured fields for both records"
    )


def test_handler_filters_keep_their_own_record_snapshot(tmp_path: Path) -> None:
    """Later handler enrichment must not leak into earlier handlers."""
    reset_manager()
    first_path = tmp_path / "first-handler.log"
    second_path = tmp_path / "second-handler.log"

    def add_first_field(record: object) -> bool:
        vars(record)["first_field"] = "first"
        return True

    def add_second_field(record: object) -> bool:
        vars(record)["second_field"] = "second"
        return True

    first_handler = (
        FileHandlerBuilder(str(first_path))
        .with_flush_after_records(1)
        .with_filters(["first"])
        .with_formatter(lambda record: repr(record["metadata"]["key_values"]))
    )
    second_handler = (
        FileHandlerBuilder(str(second_path))
        .with_flush_after_records(1)
        .with_filters(["second"])
        .with_formatter(lambda record: repr(record["metadata"]["key_values"]))
    )
    config = (
        ConfigBuilder()
        .with_version(1)
        .with_filter("first", PythonCallbackFilterBuilder(add_first_field))
        .with_filter("second", PythonCallbackFilterBuilder(add_second_field))
        .with_handler("first", first_handler)
        .with_handler("second", second_handler)
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["first", "second"])
        )
    )

    try:
        config.build_and_init()
        get_logger("app.child").info("both handlers")
        poll_file_for_text(first_path, "'first_field': 'first'", timeout=1.0)
        poll_file_for_text(second_path, "'second_field': 'second'", timeout=1.0)
    finally:
        reset_manager()

    assert first_path.read_text().strip() == "{'first_field': 'first'}", (
        "the first handler should not receive fields added by the second filter"
    )
    assert second_path.read_text().strip() == (
        "{'first_field': 'first', 'second_field': 'second'}"
    ), "the second handler should receive both fields from its filter chain"


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

    assert path.read_text().strip() == "REQ-99 authorised", (
        "formatter builders must render structured callback fields"
    )


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

    assert path.read_text().strip() == "REQ-99 authorised", (
        "registered formatters must render structured callback fields"
    )


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

    assert path.read_text().strip() == "authorised", (
        "dictConfig must resolve formatter identifiers"
    )


def test_registered_default_formatter_overrides_builtin(tmp_path: Path) -> None:
    """The registered ``default`` formatter should resolve before fallback."""
    reset_manager()
    path = tmp_path / "registered-default-formatter.log"
    handler = (
        FileHandlerBuilder(str(path))
        .with_flush_after_records(1)
        .with_formatter("default")
    )
    config = (
        ConfigBuilder()
        .with_version(1)
        .with_formatter("default", FormatterBuilder().with_format("CUSTOM:%(message)s"))
        .with_handler("output", handler)
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["output"])
        )
    )

    try:
        config.build_and_init()
        get_logger("app").info("registered default")
        poll_file_for_text(path, "CUSTOM:registered default", timeout=1.0)
    finally:
        reset_manager()

    assert path.read_text().strip() == "CUSTOM:registered default", (
        "the registered default formatter should replace the built-in formatter"
    )
