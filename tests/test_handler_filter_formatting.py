"""Integration tests for handler filters and structured formatters."""

from __future__ import annotations

import collections.abc as cabc
import contextvars
import threading
import typing as typ

import pytest

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


def _format_key_values(record: cabc.Mapping[str, object]) -> str:
    """Render the structured fields attached to a formatter record."""
    metadata = record["metadata"]
    assert isinstance(metadata, cabc.Mapping), "formatter metadata must be a mapping"
    return repr(metadata["key_values"])


def test_invalid_handler_filter_ids_do_not_create_file(
    tmp_path: Path,
) -> None:
    """Unknown filter IDs fail before the file handler is opened."""
    path = tmp_path / "invalid-handler.log"
    config = (
        ConfigBuilder()
        .with_handler("output", FileHandlerBuilder(str(path)).with_filters(["missing"]))
        .with_root_logger(LoggerConfigBuilder().with_handlers(["output"]))
    )
    with pytest.raises(KeyError, match="missing"):
        config.build_and_init()
    assert not path.exists(), "invalid handler filters must not create output files"


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
        .with_formatter(_format_key_values)
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


def test_handler_filter_runs_on_emitter_thread_with_context_and_rejects(
    tmp_path: Path,
) -> None:
    """Producer filters retain caller context and suppress rejected records."""
    reset_manager()
    path = tmp_path / "producer-filter-context.log"
    request_id: contextvars.ContextVar[str | None] = contextvars.ContextVar(
        "request_id", default=None
    )
    emitter_thread = threading.get_ident()
    observations: list[tuple[int, str | None, str]] = []

    def enrich_and_filter(record: object) -> bool:
        attributes = vars(record)
        context_id = request_id.get()
        message = typ.cast("str", attributes["msg"])
        observations.append((threading.get_ident(), context_id, message))
        if context_id is None:
            return False
        attributes["correlation_id"] = context_id
        return message != "suppressed"

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
        .with_filter("context", PythonCallbackFilterBuilder(enrich_and_filter))
        .with_handler("output", handler)
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["output"])
        )
    )

    token = request_id.set("REQ-99")
    try:
        config.build_and_init()
        child = get_logger("app.child")
        child.info("accepted")
        child.info("suppressed")
        assert get_logger("root").flush_handlers(), "root handler delivery should flush"
    finally:
        request_id.reset(token)

    assert observations == [
        (emitter_thread, "REQ-99", "accepted"),
        (emitter_thread, "REQ-99", "suppressed"),
    ], "handler filters must run on the emitter thread with its ContextVar state"
    assert path.read_text().splitlines() == ["REQ-99 accepted"], (
        "accepted records keep contextual enrichment and rejected records stay absent"
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
        .with_formatter(_format_key_values)
    )
    second_handler = (
        FileHandlerBuilder(str(second_path))
        .with_flush_after_records(1)
        .with_filters(["second"])
        .with_formatter(_format_key_values)
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


@pytest.mark.parametrize(
    "formatter_source",
    [
        pytest.param("direct", id="direct-builder"),
        pytest.param("registered", id="registered-formatter"),
    ],
)
def test_formatter_renders_structured_callback_field(
    tmp_path: Path,
    formatter_source: typ.Literal["direct", "registered"],
) -> None:
    """Direct and registered formatters render handler-filter fields."""
    reset_manager()
    path = tmp_path / f"formatter-{formatter_source}.log"
    formatter = FormatterBuilder().with_format("%(correlation_id)s %(message)s")

    def add_correlation_id(record: object) -> bool:
        vars(record)["correlation_id"] = "REQ-99"
        return True

    handler = (
        FileHandlerBuilder(str(path))
        .with_flush_after_records(1)
        .with_filters(["context"])
    )
    if formatter_source == "direct":
        handler = handler.with_formatter(formatter)
    else:
        handler = handler.with_formatter("context")

    config = ConfigBuilder().with_version(1)
    if formatter_source == "registered":
        config = config.with_formatter("context", formatter)
    config = (
        config
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
        f"{formatter_source} formatter must render structured callback fields"
    )


def test_formatter_builder_defaults_to_message_format(tmp_path: Path) -> None:
    """A formatter builder without a format renders only the message."""
    reset_manager()
    path = tmp_path / "default-formatter-builder.log"
    handler = (
        FileHandlerBuilder(str(path))
        .with_flush_after_records(1)
        .with_formatter(FormatterBuilder())
    )
    config = (
        ConfigBuilder()
        .with_version(1)
        .with_handler("output", handler)
        .with_root_logger(
            LoggerConfigBuilder().with_level("INFO").with_handlers(["output"])
        )
    )

    try:
        config.build_and_init()
        get_logger("app").info("default formatter output")
        poll_file_for_text(path, "default formatter output", timeout=1.0)
    finally:
        reset_manager()

    assert path.read_text().strip() == "default formatter output", (
        "a FormatterBuilder without an explicit format should render the message"
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
