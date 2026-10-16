"""Unit tests for structured runtime reconfiguration workflows."""

from __future__ import annotations

import re
import typing as typ

import pytest

from femtologging import (
    ConfigBuilder,
    FileHandlerBuilder,
    FormatterBuilder,
    LevelFilterBuilder,
    LoggerConfigBuilder,
    LoggerMutationBuilder,
    NameFilterBuilder,
    PythonCallbackFilterBuilder,
    RuntimeConfigBuilder,
    StreamHandlerBuilder,
    get_logger,
    reset_manager,
)
from tests.helpers import poll_file_for_text

if typ.TYPE_CHECKING:
    import collections.abc as cabc
    from pathlib import Path

    from syrupy.assertion import SnapshotAssertion


# Fragment of the error raised when a root mutation is requested through both
# ``with_root_logger()`` and ``with_logger("root", ...)``.
ROOT_OVERLAP_MESSAGE = 'with_root_logger() and with_logger("root", ...)'


@pytest.fixture(autouse=True)
def reset_logger_state() -> cabc.Iterator[None]:
    """Reset the global manager around each test."""
    reset_manager()
    yield
    reset_manager()


@pytest.fixture(name="core_config")
def fixture_core_config(reset_logger_state: None) -> None:
    """Install the baseline ``core`` logger used by the mutation tests.

    Depends on ``reset_logger_state`` so the configuration is installed after
    the manager has been reset, never before it.
    """
    (
        ConfigBuilder()
        .with_handler("stderr", StreamHandlerBuilder.stderr())
        .with_filter("lvl", LevelFilterBuilder().with_max_level("DEBUG"))
        .with_root_logger(LoggerConfigBuilder().with_level("DEBUG"))
        .with_logger(
            "core",
            LoggerConfigBuilder().with_handlers(["stderr"]).with_filters(["lvl"]),
        )
        .build_and_init()
    )


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

    handler = (
        FileHandlerBuilder(str(path))
        .with_flush_after_records(1)
        .with_filters(["context"])
        .with_formatter(
            lambda record: f"{record['metadata']['key_values']!r} {record['message']}"
        )
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


def test_runtime_filter_override_updates_only_referencing_handler(
    tmp_path: Path,
) -> None:
    """A registry-only override updates attached filters without logger mutations."""
    original_messages: list[str] = []
    replacement_messages: list[str] = []
    unaffected_messages: list[str] = []

    def original_context(record: object) -> bool:
        """Mark records handled before the filter registry changes."""
        attributes = vars(record)
        original_messages.append(typ.cast("str", attributes["msg"]))
        attributes["runtime_context"] = "original"
        return True

    def replacement_context(record: object) -> bool:
        """Replace enrichment and reject records through the overridden filter."""
        attributes = vars(record)
        message = typ.cast("str", attributes["msg"])
        replacement_messages.append(message)
        attributes["runtime_context"] = "replacement"
        return message != "rejected"

    def unaffected_context(record: object) -> bool:
        """Keep the other handler's enrichment and rejection policy intact."""
        attributes = vars(record)
        message = typ.cast("str", attributes["msg"])
        unaffected_messages.append(message)
        attributes["runtime_context"] = "unaffected"
        return message != "rejected"

    config = (
        ConfigBuilder()
        .with_version(1)
        .with_filter("context", PythonCallbackFilterBuilder(original_context))
        .with_filter("other", PythonCallbackFilterBuilder(unaffected_context))
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
    service = get_logger("service")
    other = get_logger("other")
    service.info("before")
    other.info("before")
    assert service.flush_handlers(), "initial service records should flush"
    assert other.flush_handlers(), "initial unaffected records should flush"

    RuntimeConfigBuilder().with_filter(
        "context", PythonCallbackFilterBuilder(replacement_context)
    ).apply()

    service.info("accepted")
    service.info("rejected")
    other.info("accepted")
    other.info("rejected")
    assert service.flush_handlers(), "replacement-filter records should flush"
    assert other.flush_handlers(), "unaffected-filter records should flush"

    service_output = poll_file_for_text(
        tmp_path / "service.log", "replacement accepted"
    )
    other_output = poll_file_for_text(tmp_path / "other.log", "unaffected accepted")
    assert original_messages == ["before"], (
        "the old callback must stop receiving records"
    )
    assert replacement_messages == ["accepted", "rejected"], (
        "a filter registry override must reattach the replacement to existing handlers"
    )
    assert unaffected_messages == ["before", "accepted", "rejected"], (
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
