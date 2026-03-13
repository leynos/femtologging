//! Unit tests for runtime mutation builders and apply semantics.
#![cfg(all(test, feature = "python"))]

use super::test_utils::gil_and_clean_manager;
use super::*;
use crate::{
    FemtoLevel, StreamHandlerBuilder,
    filters::{FilterBuilder, LevelFilterBuilder, NameFilterBuilder},
    manager,
};
use pyo3::Python;
use rstest::{fixture, rstest};
use serial_test::serial;

fn handler_ptrs(logger: &crate::logger::FemtoLogger) -> Vec<usize> {
    logger
        .handlers_for_test()
        .iter()
        .map(|handler| std::sync::Arc::as_ptr(handler) as *const () as usize)
        .collect()
}

#[fixture]
fn configured_core_logger(_gil_and_clean_manager: ()) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Debug);
    let filter = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
    ConfigBuilder::new()
        .with_handler("stderr", StreamHandlerBuilder::stderr())
        .with_filter("lvl", FilterBuilder::Level(filter))
        .with_root_logger(root)
        .with_logger(
            "core",
            LoggerConfigBuilder::new()
                .with_handlers(["stderr"])
                .with_filters(["lvl"]),
        )
        .build_and_init()
        .expect("initial build should succeed");
}

#[rstest]
#[serial]
fn append_handler_preserves_existing_handler_arc(_configured_core_logger: ()) {
    Python::attach(|py| {
        let logger = manager::get_logger(py, "core").expect("logger should exist");
        let before = handler_ptrs(&logger.borrow(py));

        RuntimeConfigBuilder::new()
            .with_handler("stdout", StreamHandlerBuilder::stdout())
            .with_logger(
                "core",
                LoggerMutationBuilder::new().append_handlers(["stdout"]),
            )
            .apply()
            .expect("runtime mutation should succeed");

        let after = handler_ptrs(&logger.borrow(py));
        assert_eq!(after.len(), 2, "core logger should now have two handlers");
        assert_eq!(
            before[0], after[0],
            "the existing handler arc should be preserved for unchanged ids",
        );
    });
}

#[rstest]
#[serial]
fn replace_filters_changes_live_filtering(_configured_core_logger: ()) {
    Python::attach(|py| {
        let logger = manager::get_logger(py, "core").expect("logger should exist");
        assert!(
            logger
                .borrow(py)
                .log(FemtoLevel::Error, "blocked by level")
                .is_none(),
            "the initial level filter should suppress ERROR records",
        );

        RuntimeConfigBuilder::new()
            .with_filter(
                "name",
                FilterBuilder::Name(NameFilterBuilder::new().with_prefix("core")),
            )
            .with_logger(
                "core",
                LoggerMutationBuilder::new().replace_filters(["name"]),
            )
            .apply()
            .expect("runtime mutation should succeed");

        assert!(
            logger
                .borrow(py)
                .log(FemtoLevel::Error, "allowed")
                .is_some(),
            "the replacement filter should allow the core logger to emit",
        );
    });
}

#[rstest]
#[serial]
fn unknown_removed_handler_preserves_existing_state(_configured_core_logger: ()) {
    Python::attach(|py| {
        let logger = manager::get_logger(py, "core").expect("logger should exist");
        let before = handler_ptrs(&logger.borrow(py));

        let err = RuntimeConfigBuilder::new()
            .with_logger(
                "core",
                LoggerMutationBuilder::new().remove_handlers(["missing"]),
            )
            .apply()
            .expect_err("unknown ids should be rejected");

        assert!(matches!(err, ConfigError::UnknownIds(ids) if ids == vec!["missing".to_string()]));
        assert_eq!(
            before,
            handler_ptrs(&logger.borrow(py)),
            "failed runtime mutation must leave handler state intact",
        );
    });
}

#[rstest]
#[serial]
fn replacing_shared_handler_id_updates_untouched_loggers(_gil_and_clean_manager: ()) {
    Python::attach(|py| {
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let logger_cfg = LoggerConfigBuilder::new().with_handlers(["shared"]);
        ConfigBuilder::new()
            .with_handler("shared", StreamHandlerBuilder::stderr())
            .with_root_logger(root)
            .with_logger("first", logger_cfg.clone())
            .with_logger("second", logger_cfg)
            .build_and_init()
            .expect("initial build should succeed");

        let first = manager::get_logger(py, "first").expect("first logger should exist");
        let second = manager::get_logger(py, "second").expect("second logger should exist");
        let before_first = handler_ptrs(&first.borrow(py));
        let before_second = handler_ptrs(&second.borrow(py));
        assert_eq!(
            before_first, before_second,
            "shared handler should start shared"
        );

        RuntimeConfigBuilder::new()
            .with_handler("shared", StreamHandlerBuilder::stdout())
            .apply()
            .expect("runtime handler replacement should succeed");

        let after_first = handler_ptrs(&first.borrow(py));
        let after_second = handler_ptrs(&second.borrow(py));
        assert_eq!(after_first, after_second, "replacement should stay shared");
        assert_ne!(
            before_first, after_first,
            "replacing a handler id should refresh the live shared handler arc",
        );
    });
}
