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

/// Build a config expected to fail due to duplicate IDs.
/// `setup` attaches handlers or filters before the error is triggered.
fn build_with_duplicate_ids<F>(mut logger_cfg: LoggerConfigBuilder, setup: F) -> ConfigError
where
    F: FnOnce(ConfigBuilder) -> ConfigBuilder,
{
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    setup(ConfigBuilder::new())
        .with_root_logger(root)
        .with_logger("child", logger_cfg)
        .build_and_init()
        .expect_err("build_and_init should fail for duplicate ids")
}

/// Sort IDs and compare to expected, ignoring order.
fn assert_unordered_ids(mut ids: Vec<String>, expected: &[&str]) {
    ids.sort();
    let mut expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(ids, expected);
}

#[rstest]
#[serial]
fn duplicate_handler_ids_rejected(_gil_and_clean_manager: ()) {
    let handler = StreamHandlerBuilder::stderr();
    let mut logger_cfg = LoggerConfigBuilder::new();
    logger_cfg.handlers = vec!["h".into(), "i".into(), "h".into(), "i".into()];
    let err = build_with_duplicate_ids(logger_cfg, |b| b.with_handler("h", handler));
    if let ConfigError::DuplicateHandlerIds(ids) = err {
        assert_unordered_ids(ids, &["h", "i"]);
    } else {
        panic!("expected DuplicateHandlerIds error");
    }
}

#[rstest]
#[serial]
fn duplicate_filter_ids_rejected(_gil_and_clean_manager: ()) {
    let filt = LevelFilterBuilder::new().with_max_level(FemtoLevel::Info);
    let mut logger_cfg = LoggerConfigBuilder::new();
    logger_cfg.filters = vec!["f".into(), "g".into(), "f".into(), "g".into()];
    let err = build_with_duplicate_ids(logger_cfg, |b| {
        b.with_filter("f", FilterBuilder::Level(filt))
    });
    if let ConfigError::DuplicateFilterIds(ids) = err {
        assert_unordered_ids(ids, &["f", "g"]);
    } else {
        panic!("expected DuplicateFilterIds error");
    }
}

#[rstest]
#[serial]
fn disable_existing_loggers_clears_unmentioned(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let handler = StreamHandlerBuilder::stderr();
        let filt = LevelFilterBuilder::new().with_max_level(FemtoLevel::Debug);
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let builder = ConfigBuilder::new()
            .with_handler("h", handler)
            .with_filter("f", FilterBuilder::Level(filt))
            .with_root_logger(root.clone())
            .with_logger(
                "stale",
                LoggerConfigBuilder::new()
                    .with_handlers(["h"])
                    .with_filters(["f"]),
            );
        builder
            .build_and_init()
            .expect("initial build should succeed");

        let stale = manager::get_logger(py, "stale").expect("get_logger('stale') should succeed");
        assert!(!stale.borrow(py).handlers_for_test().is_empty());

        let rebuild = ConfigBuilder::new()
            .with_root_logger(root)
            .with_disable_existing_loggers(true);
        rebuild.build_and_init().expect("rebuild should succeed");

        let stale = manager::get_logger(py, "stale").expect("get_logger('stale') should succeed");
        assert!(
            stale.borrow(py).handlers_for_test().is_empty(),
            "stale logger should be disabled",
        );
    });
}
#[rstest]
#[serial]
fn disable_existing_loggers_keeps_ancestors(_gil_and_clean_manager: ()) {
    Python::with_gil(|py| {
        let handler = StreamHandlerBuilder::stderr();
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let builder = ConfigBuilder::new()
            .with_handler("h", handler)
            .with_root_logger(root.clone())
            .with_logger("parent", LoggerConfigBuilder::new().with_handlers(["h"]))
            .with_logger("parent.child", LoggerConfigBuilder::new());
        builder
            .build_and_init()
            .expect("initial build should succeed");

        let parent =
            manager::get_logger(py, "parent").expect("get_logger('parent') should succeed");
        assert!(
            !parent.borrow(py).handlers_for_test().is_empty(),
            "ancestor logger should have a handler",
        );

        let rebuild = ConfigBuilder::new()
            .with_root_logger(root)
            .with_disable_existing_loggers(true)
            .with_logger("parent.child", LoggerConfigBuilder::new());
        rebuild.build_and_init().expect("rebuild should succeed");

        let parent =
            manager::get_logger(py, "parent").expect("get_logger('parent') should succeed");
        assert!(
            !parent.borrow(py).handlers_for_test().is_empty(),
            "ancestor logger should be retained",
        );

        let child = manager::get_logger(py, "parent.child")
            .expect("get_logger('parent.child') should succeed");
        assert!(
            child.borrow(py).handlers_for_test().is_empty(),
            "child logger should have no handlers",
        );
    });
}
