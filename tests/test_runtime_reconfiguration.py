"""Unit tests for structured runtime reconfiguration workflows."""

from __future__ import annotations

import collections.abc as cabc
import re
import typing as typ

import pytest

from femtologging import (
    ConfigBuilder,
    FileHandlerBuilder,
    FormatterBuilder,
    LoggerConfigBuilder,
    LoggerMutationBuilder,
    NameFilterBuilder,
    PythonCallbackFilterBuilder,
    RuntimeConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
)
from tests.helpers import poll_file_for_text

if typ.TYPE_CHECKING:
    from pathlib import Path

    from syrupy.assertion import SnapshotAssertion


# Fragment of the error raised when a root mutation is requested through both
# ``with_root_logger()`` and ``with_logger("root", ...)``.
ROOT_OVERLAP_MESSAGE = 'with_root_logger() and with_logger("root", ...)'


pytestmark = pytest.mark.usefixtures("reset_logger_state")


def test_runtime_builder_dict_matches_snapshot(snapshot: SnapshotAssertion) -> None:
    """The Python-facing runtime builder shape should stay stable."""
    runtime = (
        RuntimeConfigBuilder()
        .with_handler("stdout", StreamHandlerBuilder.stdout())
        .with_filter("name", NameFilterBuilder().with_prefix("core"))
        .with_root_logger(LoggerMutationBuilder().append_handlers(["stdout"]))
        .with_logger(
            "core",
            LoggerMutationBuilder()
            .with_level("ERROR")
            .replace_filters(["name"])
            .append_handlers(["stdout"]),
        )
    )
    assert runtime.as_dict() == snapshot, (
        "RuntimeConfigBuilder.as_dict() must keep the documented wire shape "
        "that dictConfig-style callers depend on"
    )


def test_runtime_apply_appends_handler_and_replaces_filters(
    core_config: None,
) -> None:
    """Happy-path runtime mutation should take effect immediately."""
    logger = get_logger("core")
    before = logger.handler_ptrs_for_test()

    (
        RuntimeConfigBuilder()
        .with_handler("stdout", StreamHandlerBuilder.stdout())
        .with_filter("name", NameFilterBuilder().with_prefix("core"))
        .with_logger(
            "core",
            LoggerMutationBuilder()
            .append_handlers(["stdout"])
            .replace_filters(["name"]),
        )
        .apply()
    )

    after = logger.handler_ptrs_for_test()
    assert after[: len(before)] == before, (
        "append_handlers must retain the pre-existing handlers in order, but "
        f"{after} does not start with {before}"
    )
    assert len(after) == len(before) + 1, (
        "append_handlers(['stdout']) must add exactly one handler, taking the "
        f"core logger from {len(before)} to {len(after)} handlers"
    )
    assert logger.log("ERROR", "now allowed") is not None, (
        "replace_filters(['name']) must swap out the DEBUG-only level filter "
        "so ERROR records reach the handlers"
    )


def test_runtime_apply_unknown_filter_preserves_previous_state(
    core_config: None,
) -> None:
    """Failed runtime mutation must leave the prior runtime configuration intact."""
    logger = get_logger("core")

    with pytest.raises(KeyError, match="missing"):
        (
            RuntimeConfigBuilder()
            .with_logger(
                "core",
                LoggerMutationBuilder().replace_filters(["missing"]),
            )
            .apply()
        )

    assert logger.log("ERROR", "still blocked") is None, (
        "expected logger.log('ERROR', ...) to stay blocked because the failed "
        "RuntimeConfigBuilder().with_logger('core', "
        "LoggerMutationBuilder().replace_filters(['missing'])).apply() must "
        "preserve the prior lvl filter"
    )


def test_runtime_invalid_handler_filter_does_not_create_file(
    core_config: None, tmp_path: Path
) -> None:
    """Invalid new handler references fail before resources or state change."""
    logger = get_logger("core")
    previous_handlers = logger.handler_ptrs_for_test()
    path = tmp_path / "invalid-runtime-handler.log"
    with pytest.raises(KeyError, match="missing"):
        (
            RuntimeConfigBuilder()
            .with_handler(
                "output", FileHandlerBuilder(str(path)).with_filters(["missing"])
            )
            .with_logger("core", LoggerMutationBuilder().replace_handlers(["output"]))
            .apply()
        )
    assert not path.exists(), "invalid runtime filters must not create output files"
    assert logger.handler_ptrs_for_test() == previous_handlers, (
        "failed runtime configuration must retain existing handlers"
    )
    assert logger.log("ERROR", "still blocked") is None, (
        "failed runtime configuration must retain existing rejection filters"
    )


def test_runtime_mutation_preserves_handler_filter_enrichment_and_rejection(
    tmp_path: Path,
) -> None:
    """Replacing logger state must retain attached handler filters."""
    path = tmp_path / "runtime-handler-filter.log"
    seen_messages: list[str] = []

    def enrich_and_reject(record: object) -> bool:
        attributes = vars(record)
        message = typ.cast("str", attributes["msg"])
        seen_messages.append(message)
        attributes["runtime_context"] = "attached"
        return message != "rejected"

    def format_record(record: cabc.Mapping[str, object]) -> str:
        metadata = record["metadata"]
        message = record["message"]
        assert isinstance(metadata, cabc.Mapping), (
            "formatter metadata must be a mapping"
        )
        assert isinstance(message, str), "formatter message must be text"
        return f"{metadata['key_values']!r} {message}"

    handler = (
        FileHandlerBuilder(str(path))
        .with_flush_after_records(1)
        .with_filters(["context"])
        .with_formatter(format_record)
    )
    (
        ConfigBuilder()
        .with_version(1)
        .with_filter("context", PythonCallbackFilterBuilder(enrich_and_reject))
        .with_handler("output", handler)
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
        .with_logger("service", LoggerConfigBuilder().with_handlers(["output"]))
        .build_and_init()
    )

    RuntimeConfigBuilder().with_logger(
        "service", LoggerMutationBuilder().with_level("DEBUG")
    ).apply()
    logger = get_logger("service")
    logger.info("accepted")
    logger.info("rejected")
    assert get_logger("root").flush_handlers(), "root handler delivery should flush"
    poll_file_for_text(path, "accepted", timeout=1.0)

    output = path.read_text()
    assert seen_messages == ["accepted", "rejected"], (
        "runtime replacement must keep the handler filter attached for all records"
    )
    assert "{'runtime_context': 'attached'} accepted" in output, (
        "the retained handler filter should continue to enrich accepted records"
    )
    assert "rejected" not in output, (
        "the retained handler filter should continue to reject matching records"
    )


def _context_filter_callback(
    messages: list[str], context_value: str, *, reject_rejected: bool
) -> cabc.Callable[[object], bool]:
    """Build a filter that records messages and adds contextual enrichment."""

    def callback(record: object) -> bool:
        attributes = vars(record)
        message = typ.cast("str", attributes["msg"])
        messages.append(message)
        attributes["runtime_context"] = context_value
        return not reject_rejected or message != "rejected"

    return callback


def test_runtime_filter_override_updates_only_referencing_handler(
    tmp_path: Path,
) -> None:
    """A registry-only override updates attached filters without logger mutations."""
    messages = {name: [] for name in ("original", "replacement", "unaffected")}
    callbacks = {
        "original": _context_filter_callback(
            messages["original"], "original", reject_rejected=False
        ),
        "replacement": _context_filter_callback(
            messages["replacement"], "replacement", reject_rejected=True
        ),
        "unaffected": _context_filter_callback(
            messages["unaffected"], "unaffected", reject_rejected=True
        ),
    }

    config = (
        ConfigBuilder()
        .with_version(1)
        .with_filter("context", PythonCallbackFilterBuilder(callbacks["original"]))
        .with_filter("other", PythonCallbackFilterBuilder(callbacks["unaffected"]))
        .with_root_logger(LoggerConfigBuilder().with_level("INFO"))
    )
    formatter = FormatterBuilder().with_format("%(runtime_context)s %(message)s")
    for logger_name, filter_id in (("service", "context"), ("other", "other")):
        config = config.with_handler(
            logger_name,
            FileHandlerBuilder(str(tmp_path / f"{logger_name}.log"))
            .with_flush_after_records(1)
            .with_filters([filter_id])
            .with_formatter(formatter),
        ).with_logger(logger_name, LoggerConfigBuilder().with_handlers([logger_name]))
    config.build_and_init()
    loggers = {name: get_logger(name) for name in ("service", "other")}
    loggers["service"].info("before")
    loggers["other"].info("before")
    assert loggers["service"].flush_handlers(), "initial service records should flush"
    assert loggers["other"].flush_handlers(), "initial unaffected records should flush"

    RuntimeConfigBuilder().with_filter(
        "context", PythonCallbackFilterBuilder(callbacks["replacement"])
    ).apply()

    loggers["service"].info("accepted")
    loggers["service"].info("rejected")
    loggers["other"].info("accepted")
    loggers["other"].info("rejected")
    assert loggers["service"].flush_handlers(), (
        "replacement-filter records should flush"
    )
    assert loggers["other"].flush_handlers(), "unaffected-filter records should flush"

    service_output = poll_file_for_text(
        tmp_path / "service.log", "replacement accepted"
    )
    other_output = poll_file_for_text(tmp_path / "other.log", "unaffected accepted")
    assert messages["original"] == ["before"], (
        "the old callback must stop receiving records"
    )
    assert messages["replacement"] == ["accepted", "rejected"], (
        "a filter registry override must reattach the replacement to existing handlers"
    )
    assert messages["unaffected"] == ["before", "accepted", "rejected"], (
        "handlers using another filter ID must keep their original callback"
    )
    assert service_output.splitlines() == ["original before", "replacement accepted"], (
        "the replacement must enrich accepted records and suppress rejected records"
    )
    assert other_output.splitlines() == ["unaffected before", "unaffected accepted"], (
        "the unrelated handler must preserve both enrichment and rejection"
    )


def test_runtime_apply_rejects_conflicting_collection_modes(
    core_config: None,
) -> None:
    """One logger mutation may not submit conflicting handler mutations."""
    with pytest.raises(
        ValueError,
        match=re.escape(
            "conflicting mutations to the handler collection were requested"
        ),
    ):
        (
            RuntimeConfigBuilder()
            .with_logger(
                "core",
                LoggerMutationBuilder()
                .append_handlers(["stdout"])
                .replace_handlers(["stderr"]),
            )
            .apply()
        )


def test_runtime_apply_rejects_root_name_overlap(core_config: None) -> None:
    """Root mutations must not be provided through both root builder paths."""
    with pytest.raises(
        ValueError,
        match=re.escape(ROOT_OVERLAP_MESSAGE),
    ):
        (
            RuntimeConfigBuilder()
            .with_root_logger(LoggerMutationBuilder().with_level("ERROR"))
            .with_logger(
                "root", LoggerMutationBuilder().with_propagate(propagate=False)
            )
            .apply()
        )
