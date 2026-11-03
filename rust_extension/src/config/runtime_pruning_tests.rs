//! Regression tests for runtime registry pruning.
//!
//! These tests pin the identifier-level semantics of the reachability rule:
//! which definitions a successful mutation publishes, which it rejects, and
//! what a rejection leaves behind. Object lifetimes are covered separately in
//! [`super::runtime_lifecycle_tests`].
#![cfg(all(test, feature = "python"))]

use super::{
    ConfigBuilder, ConfigError, LoggerConfigBuilder, LoggerMutationBuilder, RuntimeConfigBuilder,
    test_utils::{
        gil_and_clean_manager,
        prune_test_support::{configured_core_logger, published_filters, published_handlers},
    },
};
use crate::{
    FemtoLevel, FileHandlerBuilder, StreamHandlerBuilder,
    filters::{FilterBuilder, LevelFilterBuilder, NameFilterBuilder},
};
use pyo3::Python;
use rstest::rstest;
use serial_test::serial;

#[rstest]
#[serial]
fn unused_definition_rejected_before_build(configured_core_logger: Result<(), ConfigError>) {
    configured_core_logger.expect("initial build should succeed");
    let dir = tempfile::tempdir().expect("tempdir must create a temporary directory");
    let path = dir.path().join("orphan.log");

    Python::attach(|_py| {
        let before = published_handlers();

        let err = RuntimeConfigBuilder::new()
            .with_handler("orphan", FileHandlerBuilder::new(path.clone()))
            .apply()
            .expect_err("a definition no logger attaches must be rejected");

        assert!(
            matches!(err, ConfigError::InvalidMutation(ref message)
                if message.contains("orphan")),
            "the rejection must name the unreferenced handler, got {err}",
        );
        assert_eq!(
            before,
            published_handlers(),
            "a rejected mutation must not change the published handler registry",
        );
        assert!(
            !path.exists(),
            "rejection must happen before construction, so no file may be created",
        );
    });
}

#[rstest]
#[serial]
fn unused_filter_definition_rejected_before_build(configured_core_logger: Result<(), ConfigError>) {
    configured_core_logger.expect("initial build should succeed");

    let err = RuntimeConfigBuilder::new()
        .with_filter(
            "orphan",
            FilterBuilder::Name(NameFilterBuilder::new().with_prefix("core")),
        )
        .apply()
        .expect_err("a filter definition no logger attaches must be rejected");

    assert!(
        matches!(err, ConfigError::InvalidMutation(ref message)
            if message.contains("filters") && message.contains("orphan")),
        "the rejection must name the unreferenced filter, got {err}",
    );
    assert_eq!(
        (1, vec!["lvl".to_string()]),
        published_filters(),
        "a rejected mutation must not change the published filter registry",
    );
}

#[rstest]
#[serial]
fn empty_mutation_leaves_registries_unchanged(configured_core_logger: Result<(), ConfigError>) {
    configured_core_logger.expect("initial build should succeed");

    Python::attach(|_py| {
        let before_handlers = published_handlers();
        let before_filters = published_filters();

        RuntimeConfigBuilder::new()
            .apply()
            .expect("an empty mutation must stay a successful no-op");

        assert_eq!(
            before_handlers,
            published_handlers(),
            "an empty mutation must not prune reachable handlers",
        );
        assert_eq!(
            before_filters,
            published_filters(),
            "an empty mutation must not prune reachable filters",
        );
    });
}

#[rstest]
#[serial]
fn define_and_attach_in_one_transaction_publishes_definition(
    configured_core_logger: Result<(), ConfigError>,
) {
    configured_core_logger.expect("initial build should succeed");

    Python::attach(|_py| {
        RuntimeConfigBuilder::new()
            .with_handler("stdout", StreamHandlerBuilder::stdout())
            .with_logger(
                "core",
                LoggerMutationBuilder::new().append_handlers(["stdout"]),
            )
            .apply()
            .expect("attaching the definition in the same transaction must succeed");

        assert_eq!(
            (2, vec!["stderr".to_string(), "stdout".to_string()]),
            published_handlers(),
            "the attached definition must be published alongside the existing one",
        );
    });
}

#[rstest]
#[serial]
fn removing_last_handler_reference_prunes_definition(
    configured_core_logger: Result<(), ConfigError>,
) {
    configured_core_logger.expect("initial build should succeed");

    Python::attach(|_py| {
        RuntimeConfigBuilder::new()
            .with_logger(
                "core",
                LoggerMutationBuilder::new().remove_handlers(["stderr"]),
            )
            .apply()
            .expect("removing the last reference must succeed");

        assert_eq!(
            (0, Vec::new()),
            published_handlers(),
            "a handler whose last reference was removed must be pruned",
        );
    });
}

#[rstest]
#[serial]
fn removing_one_of_two_references_keeps_definition(_gil_and_clean_manager: ()) {
    Python::attach(|_py| {
        let logger_cfg = LoggerConfigBuilder::new().with_handlers(["shared"]);
        ConfigBuilder::new()
            .with_handler("shared", StreamHandlerBuilder::stderr())
            .with_root_logger(LoggerConfigBuilder::new())
            .with_logger("first", logger_cfg.clone())
            .with_logger("second", logger_cfg)
            .build_and_init()
            .expect("initial build should succeed");

        RuntimeConfigBuilder::new()
            .with_logger(
                "first",
                LoggerMutationBuilder::new().remove_handlers(["shared"]),
            )
            .apply()
            .expect("removing one of two references must succeed");

        assert_eq!(
            (1, vec!["shared".to_string()]),
            published_handlers(),
            "a definition another logger still references must survive pruning",
        );
    });
}

#[rstest]
#[serial]
fn detaching_and_redefining_same_id_is_rejected(configured_core_logger: Result<(), ConfigError>) {
    configured_core_logger.expect("initial build should succeed");

    Python::attach(|_py| {
        let err = RuntimeConfigBuilder::new()
            .with_handler("stderr", StreamHandlerBuilder::stdout())
            .with_logger("core", LoggerMutationBuilder::new().clear_handlers())
            .apply()
            .expect_err("detaching and redefining one id at once must be rejected");

        assert!(
            matches!(err, ConfigError::InvalidMutation(ref message)
                if message.contains("stderr")),
            "the rejection must name the detached identifier, got {err}",
        );
        assert_eq!(
            (1, vec!["stderr".to_string()]),
            published_handlers(),
            "a rejected replacement must leave the registry untouched",
        );
    });
}

#[rstest]
#[serial]
fn bootstrap_prunes_definition_no_logger_references(_gil_and_clean_manager: ()) {
    Python::attach(|_py| {
        ConfigBuilder::new()
            .with_handler("attached", StreamHandlerBuilder::stderr())
            .with_handler("unattached", StreamHandlerBuilder::stdout())
            .with_filter(
                "filter",
                FilterBuilder::Level(LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug)),
            )
            .with_root_logger(
                LoggerConfigBuilder::new()
                    .with_handlers(["attached"])
                    .with_filters(["filter"]),
            )
            .build_and_init()
            .expect("bootstrap must still build every declared definition");

        assert_eq!(
            (1, vec!["attached".to_string()]),
            published_handlers(),
            "bootstrap must publish only reachable handler definitions",
        );
        assert_eq!(
            (1, vec!["filter".to_string()]),
            published_filters(),
            "bootstrap must publish only reachable filter definitions",
        );
    });
}

#[rstest]
#[serial]
fn handler_filter_definition_survives_only_while_its_handler_is_attached(
    _gil_and_clean_manager: (),
) {
    ConfigBuilder::new()
        .with_handler(
            "filtered",
            StreamHandlerBuilder::stderr().with_filters(["handler-filter"]),
        )
        .with_filter(
            "handler-filter",
            FilterBuilder::Name(NameFilterBuilder::new().with_prefix("core")),
        )
        .with_root_logger(LoggerConfigBuilder::new().with_handlers(["filtered"]))
        .build_and_init()
        .expect("bootstrap must retain filters referenced by an attached handler");

    assert_eq!(
        (1, vec!["handler-filter".to_string()]),
        published_filters(),
        "a handler-level filter is reachable through its attached handler",
    );

    RuntimeConfigBuilder::new()
        .with_root_logger(LoggerMutationBuilder::new().clear_handlers())
        .apply()
        .expect("detaching the handler must prune its filter definition");

    assert_eq!(
        (0, Vec::new()),
        published_filters(),
        "a filter referenced only by the removed handler must be released",
    );
}

#[rstest]
#[serial]
fn unknown_definition_id_takes_precedence_over_unreachable_rejection(
    configured_core_logger: Result<(), ConfigError>,
) {
    configured_core_logger.expect("initial build should succeed");

    Python::attach(|_py| {
        let err = RuntimeConfigBuilder::new()
            .with_handler("orphan", StreamHandlerBuilder::stdout())
            .with_logger(
                "core",
                LoggerMutationBuilder::new().append_handlers(["missing"]),
            )
            .apply()
            .expect_err("an unknown attachment id must be rejected");

        assert!(
            matches!(err, ConfigError::UnknownIds(ref ids) if ids == &["missing".to_string()]),
            "UnknownIds must take precedence over the unreferenced-definition rejection, got {err}",
        );
    });
}
