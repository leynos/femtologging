//! Unit tests for runtime mutation builders and apply semantics.
#![cfg(all(test, feature = "python"))]

use super::test_utils::{gil_and_clean_manager, prune_test_support::configured_core_logger};
use super::*;
use crate::{
    FemtoLevel, StreamHandlerBuilder,
    filters::{FilterBuilder, LevelFilterBuilder, NameFilterBuilder},
    manager,
};
use pyo3::Python;
use rstest::rstest;
use serial_test::serial;
use std::sync::{Arc, Barrier};

fn handler_ptrs(logger: &crate::logger::FemtoLogger) -> Vec<usize> {
    logger
        .handlers_for_test()
        .iter()
        .map(|handler| std::sync::Arc::as_ptr(handler) as *const () as usize)
        .collect()
}

#[test]
fn same_mode_payload_conflict_has_an_accurate_error() {
    let error = RuntimeConfigBuilder::new()
        .with_logger(
            "core",
            LoggerMutationBuilder::new()
                .append_handlers(["first"])
                .append_handlers(["second"]),
        )
        .apply()
        .expect_err("different handler payloads should conflict");

    assert!(matches!(
        error,
        ConfigError::InvalidMutation(message)
            if message == "core: conflicting mutations to the handler collection were requested"
    ));
}

#[rstest]
#[serial]
fn append_handler_preserves_existing_handler_arc(configured_core_logger: Result<(), ConfigError>) {
    configured_core_logger.expect("initial build should succeed");
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
fn concurrent_applies_preserve_both_handler_updates(
    configured_core_logger: Result<(), ConfigError>,
) {
    configured_core_logger.expect("initial build should succeed");

    let start = Arc::new(Barrier::new(3));
    let first = RuntimeConfigBuilder::new()
        .with_handler("first", StreamHandlerBuilder::stdout())
        .with_logger(
            "core",
            LoggerMutationBuilder::new().append_handlers(["first"]),
        );
    let second = RuntimeConfigBuilder::new()
        .with_handler("second", StreamHandlerBuilder::stdout())
        .with_logger(
            "core",
            LoggerMutationBuilder::new().append_handlers(["second"]),
        );

    std::thread::scope(|scope| {
        let first_start = Arc::clone(&start);
        let first_apply = scope.spawn(move || {
            first_start.wait();
            first.apply()
        });
        let second_start = Arc::clone(&start);
        let second_apply = scope.spawn(move || {
            second_start.wait();
            second.apply()
        });

        start.wait();
        first_apply
            .join()
            .expect("first apply thread should not panic")
            .expect("first runtime mutation should succeed");
        second_apply
            .join()
            .expect("second apply thread should not panic")
            .expect("second runtime mutation should succeed");
    });

    Python::attach(|py| {
        let logger = manager::get_logger(py, "core").expect("logger should exist");
        assert_eq!(
            handler_ptrs(&logger.borrow(py)).len(),
            3,
            "both serialized mutations should retain their new handlers",
        );
    });
}

#[rstest]
#[serial]
fn replace_filters_changes_live_filtering(configured_core_logger: Result<(), ConfigError>) {
    configured_core_logger.expect("initial build should succeed");
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
    configured_core_logger: Result<(), ConfigError>,
) {
    configured_core_logger.expect("initial build should succeed");
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
#[case::append_handler("append handler", |b: RuntimeConfigBuilder| {
    b.with_handler("stdout", StreamHandlerBuilder::stdout())
        .with_logger(
            "orphan",
            LoggerMutationBuilder::new().append_handlers(["stdout"]),
        )
})]
#[case::remove_handler("remove handler", |b: RuntimeConfigBuilder| {
    b.with_logger(
        "orphan",
        LoggerMutationBuilder::new().remove_handlers(["stdout"]),
    )
})]
#[case::append_filter("append filter", |b: RuntimeConfigBuilder| {
    let filter = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
    b.with_filter("lvl", FilterBuilder::Level(filter)).with_logger(
        "orphan",
        LoggerMutationBuilder::new().append_filters(["lvl"]),
    )
})]
#[case::remove_filter("remove filter", |b: RuntimeConfigBuilder| {
    b.with_logger(
        "orphan",
        LoggerMutationBuilder::new().remove_filters(["lvl"]),
    )
})]
#[serial]
fn mutation_requires_runtime_metadata(
    _gil_and_clean_manager: (),
    #[case] _kind: &str,
    #[case] mutate: fn(RuntimeConfigBuilder) -> RuntimeConfigBuilder,
) {
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
fn empty_append_and_remove_allow_missing_runtime_metadata(_gil_and_clean_manager: ()) {
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
