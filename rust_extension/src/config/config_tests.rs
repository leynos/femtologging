//! Unit tests for configuration builders.
#![cfg(all(test, feature = "python"))]

use super::*;
use crate::config::ConfigError;
use crate::filters::{FilterBuilder, LevelFilterBuilder};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::handlers::{HandlerBuildError, HandlerBuilderTrait};
use crate::log_record::FemtoLogRecord;
use crate::manager;
use crate::{FemtoLevel, StreamHandlerBuilder};
use parking_lot::Mutex;
use pyo3::Python;
use rstest::{fixture, rstest};
use serial_test::serial;
use std::any::Any;
use std::sync::Arc;

#[derive(Clone, Default)]
pub(crate) struct CollectingHandler {
    records: Arc<Mutex<Vec<FemtoLogRecord>>>,
}

impl CollectingHandler {
    pub(crate) fn collected(&self) -> Vec<FemtoLogRecord> {
        self.records.lock().clone()
    }
}

impl FemtoHandlerTrait for CollectingHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.records.lock().push(record);
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
pub(crate) struct CollectingHandlerBuilder {
    handler: CollectingHandler,
}

impl CollectingHandlerBuilder {
    pub(crate) fn new() -> Self {
        Self {
            handler: CollectingHandler::default(),
        }
    }

    pub(crate) fn handle(&self) -> CollectingHandler {
        self.handler.clone()
    }
}

impl HandlerBuilderTrait for CollectingHandlerBuilder {
    type Handler = CollectingHandler;

    fn build_inner(&self) -> Result<Self::Handler, HandlerBuildError> {
        Ok(self.handler.clone())
    }
}

#[fixture]
pub fn gil_and_clean_manager() {
    Python::with_gil(|_| manager::reset_manager());
}

pub fn builder_with_root(root: LoggerConfigBuilder) -> ConfigBuilder {
    ConfigBuilder::new()
        .with_handler("h", StreamHandlerBuilder::stderr())
        .with_root_logger(root)
}

#[fixture]
pub fn base_logger_builder() -> (ConfigBuilder, LoggerConfigBuilder) {
    let root = LoggerConfigBuilder::new().with_level(FemtoLevel::Info);
    let builder = builder_with_root(root.clone());
    (builder, root)
}

pub fn assert_handler_count(py: Python<'_>, name: &str, expected: usize, reason: &str) {
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

mod config_tests {
    include!("config_tests/submods.rs");
}
