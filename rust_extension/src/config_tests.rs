//! Unit tests for configuration builders.

use super::*;
use crate::{
    filters::{FilterBuilder, LevelFilterBuilder},
    manager,
};
use pyo3::Python;
use rstest::rstest;
use serial_test::serial;
use std::sync::Arc;

#[rstest]
fn build_rejects_invalid_version() {
    let builder = ConfigBuilder::new().with_version(2);
    assert!(builder.build_and_init().is_err());
}

#[rstest]
fn build_rejects_missing_root() {
    let builder = ConfigBuilder::new();
    assert!(builder.build_and_init().is_err());
}

#[rstest]
fn build_accepts_default_version() {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = ConfigBuilder::new().with_root_logger(root);
    assert!(builder.build_and_init().is_ok());
}

#[rstest]
#[serial]
fn shared_handler_attached_once() {
    Python::with_gil(|py| {
        manager::reset_manager();
        let handler = StreamHandlerBuilder::stderr();
        let logger_cfg = LoggerConfigBuilder::new().with_handlers(["h"]);
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
        let builder = ConfigBuilder::new()
            .with_handler("h", handler)
            .with_root_logger(root)
            .with_logger("first", logger_cfg.clone())
            .with_logger("second", logger_cfg);
        builder.build_and_init().expect("build should succeed");
        let first = manager::get_logger(py, "first").unwrap();
        let second = manager::get_logger(py, "second").unwrap();
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
fn unknown_handler_id_rejected() {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let logger_cfg = LoggerConfigBuilder::new().with_handlers(["missing"]);
    let builder = ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder.build_and_init().unwrap_err();
    assert!(matches!(err, ConfigError::UnknownHandlerId(id) if id == "missing"));
}

#[rstest]
#[serial]
fn unknown_filter_id_rejected() {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let logger_cfg = LoggerConfigBuilder::new().with_filters(["missing"]);
    let builder = ConfigBuilder::new()
        .with_root_logger(root)
        .with_logger("child", logger_cfg);
    let err = builder.build_and_init().unwrap_err();
    assert!(matches!(err, ConfigError::UnknownFilterId(id) if id == "missing"));
}

#[rstest]
#[serial]
fn level_filter_blocks_records() {
    Python::with_gil(|py| {
        manager::reset_manager();
        let filter =
            FilterBuilder::Level(LevelFilterBuilder::new().with_max_level(FemtoLevel::Info));
        let logger_cfg = LoggerConfigBuilder::new().with_filters(["f"]);
        let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Debug);
        let builder = ConfigBuilder::new()
            .with_filter("f", filter)
            .with_root_logger(root)
            .with_logger("child", logger_cfg);
        builder.build_and_init().expect("build should succeed");
        let logger = manager::get_logger(py, "child").unwrap();
        assert!(logger.borrow(py).log(FemtoLevel::Info, "ok").is_some());
        assert!(logger.borrow(py).log(FemtoLevel::Error, "nope").is_none());
    });
}
