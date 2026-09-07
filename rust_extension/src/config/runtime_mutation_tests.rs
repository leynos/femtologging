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
        .map(|handler| std::sync::Arc::as_ptr(handler).cast::<()>() as usize)
        .collect()
}

#[fixture]
fn configured_core_logger(
    #[from(gil_and_clean_manager)] manager_reset: (),
) -> Result<(), ConfigError> {
    let () = manager_reset;
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
}

#[rstest]
#[serial]
fn append_handler_preserves_existing_handler_arc(
    #[from(configured_core_logger)] configured_core_logger_result: Result<(), ConfigError>,
) {
    configured_core_logger_result.expect("initial build should succeed");
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
        let before_handler = before.first().expect("existing handler should be present");
        let after_handler = after.first().expect("first handler should be present");
        assert_eq!(
            before_handler, after_handler,
            "the existing handler arc should be preserved for unchanged ids",
        );
    });
}

#[rstest]
#[serial]
fn replace_filters_changes_live_filtering(
    #[from(configured_core_logger)] configured_core_logger_result: Result<(), ConfigError>,
) {
    configured_core_logger_result.expect("initial build should succeed");
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
fn unknown_removed_handler_preserves_existing_state(
    #[from(configured_core_logger)] configured_core_logger_result: Result<(), ConfigError>,
) {
    configured_core_logger_result.expect("initial build should succeed");
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

        assert!(matches!(err, ConfigError::UnknownIds(ids) if ids == vec!["missing".to_owned()]));
        assert_eq!(
            before,
            handler_ptrs(&logger.borrow(py)),
            "failed runtime mutation must leave handler state intact",
        );
    });
}

#[rstest]
#[case::append_handler(|b: RuntimeConfigBuilder| {
    b.with_handler("stdout", StreamHandlerBuilder::stdout())
        .with_logger(
            "orphan",
            LoggerMutationBuilder::new().append_handlers(["stdout"]),
        )
})]
#[case::remove_handler(|b: RuntimeConfigBuilder| {
    b.with_logger(
        "orphan",
        LoggerMutationBuilder::new().remove_handlers(["stdout"]),
    )
})]
#[case::append_filter(|b: RuntimeConfigBuilder| {
    let filter = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
    b.with_filter("lvl", FilterBuilder::Level(filter)).with_logger(
        "orphan",
        LoggerMutationBuilder::new().append_filters(["lvl"]),
    )
})]
#[case::remove_filter(|b: RuntimeConfigBuilder| {
    b.with_logger(
        "orphan",
        LoggerMutationBuilder::new().remove_filters(["lvl"]),
    )
})]
#[serial]
fn mutation_requires_runtime_metadata(
    #[from(gil_and_clean_manager)] manager_reset: (),
    #[case] mutate: fn(RuntimeConfigBuilder) -> RuntimeConfigBuilder,
) {
    let () = manager_reset;
    Python::attach(|py| {
        let _logger = manager::get_logger(py, "orphan").expect("logger should exist");

        let err = mutate(RuntimeConfigBuilder::new())
            .apply()
            .expect_err("mutation should reject entities without runtime metadata");

        assert!(matches!(
            err,
            ConfigError::InvalidMutation(message)
                if message == "orphan: logger has no runtime metadata; Append/Remove require prior build_and_init()"
        ));
    });
}

#[rstest]
#[serial]
fn empty_append_and_remove_allow_missing_runtime_metadata(
    #[from(gil_and_clean_manager)] manager_reset: (),
) {
    let () = manager_reset;
    Python::attach(|py| {
        let logger = manager::get_logger(py, "orphan").expect("logger should exist");

        RuntimeConfigBuilder::new()
            .with_logger(
                "orphan",
                LoggerMutationBuilder::new()
                    .append_handlers(Vec::<&str>::new())
                    .remove_filters(Vec::<&str>::new()),
            )
            .apply()
            .expect("empty append/remove should act like a no-op without runtime metadata");

        assert!(
            handler_ptrs(&logger.borrow(py)).is_empty(),
            "a no-op runtime mutation should not attach handlers",
        );
    });
}

#[rstest]
#[serial]
fn replacing_shared_handler_id_updates_untouched_loggers(
    #[from(gil_and_clean_manager)] manager_reset: (),
) {
    let () = manager_reset;
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
