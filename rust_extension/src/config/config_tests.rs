//! Unit tests for configuration builders.
#![cfg(all(test, feature = "python"))]

use super::*;
use crate::config::ConfigError;
use crate::filters::{FilterBuilder, LevelFilterBuilder};
use crate::manager;
use crate::{FemtoLevel, StreamHandlerBuilder};
use pyo3::Python;
use rstest::{fixture, rstest};
use serial_test::serial;
use std::sync::Arc;

#[fixture]
fn gil_and_clean_manager() {
    Python::with_gil(|_| manager::reset_manager());
}

fn builder_with_root(root: LoggerConfigBuilder) -> ConfigBuilder {
    ConfigBuilder::new()
        .with_handler("h", StreamHandlerBuilder::stderr())
        .with_root_logger(root)
}

#[fixture]
fn base_logger_builder() -> (ConfigBuilder, LoggerConfigBuilder) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = builder_with_root(root.clone());
    (builder, root)
}

fn assert_handler_count(py: Python<'_>, name: &str, expected: usize, reason: &str) {
    // Fetch a logger and assert it exposes the expected number of handlers.
    let msg = format!("get_logger('{name}') should succeed");
    let logger = manager::get_logger(py, name).expect(&msg);
    let count = logger.borrow(py).handlers_for_test().len();
    assert_eq!(count, expected, "{}", reason);
}

#[rstest]
fn build_rejects_invalid_version() {
    let builder = ConfigBuilder::new().with_version(2);
    let err = builder
        .build_and_init()
        .expect_err("version 2 should be rejected");
    assert!(matches!(err, ConfigError::UnsupportedVersion(2)));
}

#[rstest]
fn build_rejects_missing_root() {
    let builder = ConfigBuilder::new();
    let err = builder
        .build_and_init()
        .expect_err("root logger is required");
    assert!(matches!(err, ConfigError::MissingRootLogger));
}

#[rstest]
#[serial]
fn build_accepts_default_version(_gil_and_clean_manager: ()) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = ConfigBuilder::new().with_root_logger(root);
    assert!(builder.build_and_init().is_ok());
}

#[rstest]
#[serial]
fn shared_handler_attached_once(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let handler = StreamHandlerBuilder::stderr();
        let logger_cfg = LoggerConfigBuilder::new().with_handlers(["h"]);
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let builder = ConfigBuilder::new()
            .with_handler("h", handler)
            .with_root_logger(root)
            .with_logger("first", logger_cfg.clone())
            .with_logger("second", logger_cfg);
        builder.build_and_init().expect("build should succeed");
        let first = manager::get_logger(py, "first").expect("get_logger('first') should succeed");
        let second =
            manager::get_logger(py, "second").expect("get_logger('second') should succeed");
        let h1 = first.borrow(py).handlers_for_test();
        let h2 = second.borrow(py).handlers_for_test();
        assert!(
            Arc::ptr_eq(&h1[0], &h2[0]),
            "handler Arc pointers should be shared"
        );
    });
}

#[rstest]
#[serial]
fn unknown_handler_id_rejected(_gil_and_clean_manager: ()) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let logger_cfg = LoggerConfigBuilder::new().with_handlers(["missing"]);
    let builder = ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for unknown handler id");
    assert!(matches!(err, ConfigError::UnknownId(id) if id == "missing"));
}

#[rstest]
#[serial]
fn reconfig_with_unknown_filter_preserves_existing_filters(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let filt = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
        let builder = ConfigBuilder::new()
            .with_filter("lvl", FilterBuilder::Level(filt))
            .with_root_logger(root.clone())
            .with_logger("core", LoggerConfigBuilder::new().with_filters(["lvl"]));
        builder
            .build_and_init()
            .expect("initial build should succeed");
        let logger = manager::get_logger(py, "core").expect("get_logger('core') should succeed");
        assert!(logger.borrow(py).log(FemtoLevel::Error, "drop").is_none());
        let bad = ConfigBuilder::new()
            .with_root_logger(root)
            .with_logger("core", LoggerConfigBuilder::new().with_filters(["missing"]));
        assert!(bad.build_and_init().is_err());
        assert!(logger
            .borrow(py)
            .log(FemtoLevel::Error, "still drop")
            .is_none());
    });
}

#[rstest]
#[serial]
fn unknown_filter_id_rejected(_gil_and_clean_manager: ()) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let logger_cfg = LoggerConfigBuilder::new().with_filters(["missing"]);
    let builder = ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for unknown filter id");
    assert!(matches!(err, ConfigError::UnknownId(id) if id == "missing"));
}

#[rstest]
#[serial]
fn duplicate_handler_ids_rejected(_gil_and_clean_manager: ()) {
    let handler = StreamHandlerBuilder::stderr();
    let mut logger_cfg = LoggerConfigBuilder::new();
    logger_cfg.handlers = vec!["h".into(), "h".into()];
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = ConfigBuilder::new()
        .with_handler("h", handler)
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for duplicate handler ids");
    assert!(matches!(err, ConfigError::DuplicateHandlerIds(ids) if ids == vec!["h".to_string()]));
}

#[rstest]
#[serial]
fn duplicate_filter_ids_rejected(_gil_and_clean_manager: ()) {
    let filt = LevelFilterBuilder::new().with_max_level(FemtoLevel::Info);
    let mut logger_cfg = LoggerConfigBuilder::new();
    logger_cfg.filters = vec!["f".into(), "f".into()];
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = ConfigBuilder::new()
        .with_filter("f", FilterBuilder::Level(filt))
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder
        .build_and_init()
        .expect_err("build_and_init should fail for duplicate filter ids");
    assert!(matches!(err, ConfigError::DuplicateFilterIds(ids) if ids == vec!["f".to_string()]));
}
#[rstest]
#[serial]
fn disable_existing_loggers_clears_unmentioned(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::with_gil(|py| {
        let (builder, root) = base_logger_builder;
        let filt = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
        let builder = builder
            .with_filter("f", FilterBuilder::Level(filt))
            .with_logger(
                "stale",
                LoggerConfigBuilder::new()
                    .with_handlers(["h"])
                    .with_filters(["f"]),
            );
        builder
            .build_and_init()
            .expect("initial build should succeed");

        assert_handler_count(py, "stale", 1, "stale logger should start active");

        let rebuild = ConfigBuilder::new()
            .with_root_logger(root)
            .with_disable_existing_loggers(true);
        rebuild.build_and_init().expect("rebuild should succeed");

        assert_handler_count(py, "stale", 0, "stale logger should be disabled");
    });
}

#[rstest]
#[serial]
fn disable_existing_loggers_keeps_ancestors(
    _gil_and_clean_manager: (),
    base_logger_builder: (ConfigBuilder, LoggerConfigBuilder),
) {
    Python::with_gil(|py| {
        let (builder, root) = base_logger_builder;
        let builder = builder
            .with_logger(
                "grandparent",
                LoggerConfigBuilder::new().with_handlers(["h"]),
            )
            .with_logger(
                "grandparent.parent",
                LoggerConfigBuilder::new().with_handlers(["h"]),
            );
        builder
            .build_and_init()
            .expect("initial build should succeed");

        assert_handler_count(py, "grandparent", 1, "grandparent should start active");
        assert_handler_count(py, "grandparent.parent", 1, "parent should start active");

        let rebuild = builder_with_root(root)
            .with_logger(
                "grandparent.parent.child",
                LoggerConfigBuilder::new().with_handlers(["h"]),
            )
            .with_disable_existing_loggers(true);
        rebuild.build_and_init().expect("rebuild should succeed");

        assert_handler_count(py, "grandparent", 1, "ancestor logger should remain active");
        assert_handler_count(
            py,
            "grandparent.parent",
            1,
            "ancestor logger should remain active",
        );
        assert_handler_count(
            py,
            "grandparent.parent.child",
            1,
            "child logger should retain its handler",
        );

        let logging = py.import("logging").expect("import logging");
        let py_child = logging
            .call_method1("getLogger", ("grandparent.parent.child",))
            .expect("getLogger('grandparent.parent.child') should succeed");
        let py_handlers = py_child
            .getattr("handlers")
            .expect("child logger should expose handlers");
        assert_eq!(
            py_handlers.len().unwrap(),
            1,
            "child logger should retain its handler",
        );
    });
}
