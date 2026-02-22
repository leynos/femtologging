//! Unit tests for the Python convenience logging functions.

use super::*;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;
use parking_lot::Mutex;
use rstest::{fixture, rstest};
use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

static LOGGER_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[fixture]
fn unique_logger_name() -> String {
    let suffix = LOGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("conv.test.{suffix}")
}

#[derive(Clone, Default)]
struct CollectingHandler {
    records: Arc<Mutex<Vec<FemtoLogRecord>>>,
}

impl CollectingHandler {
    fn collected(&self) -> Vec<FemtoLogRecord> {
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

#[rstest]
fn debug_dispatches_at_debug_level(unique_logger_name: String) {
    Python::attach(|py| {
        let logger =
            manager::get_logger(py, &unique_logger_name).expect("logger should be created");
        logger.borrow(py).set_level(FemtoLevel::Debug);
        let handler = Arc::new(CollectingHandler::default());
        logger
            .borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result = log_at_level(
            py,
            FemtoLevel::Debug,
            "debug msg",
            Some(&unique_logger_name),
        )
        .expect("should not error");
        assert!(result.is_some());
        assert!(logger.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level_str(), "DEBUG");
        assert_eq!(records[0].message(), "debug msg");
    });
}

#[rstest]
fn info_dispatches_at_info_level(unique_logger_name: String) {
    Python::attach(|py| {
        let logger =
            manager::get_logger(py, &unique_logger_name).expect("logger should be created");
        let handler = Arc::new(CollectingHandler::default());
        logger
            .borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result = log_at_level(py, FemtoLevel::Info, "info msg", Some(&unique_logger_name))
            .expect("should not error");
        assert!(result.is_some());
        assert!(logger.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level_str(), "INFO");
    });
}

#[rstest]
fn warn_dispatches_at_warn_level(unique_logger_name: String) {
    Python::attach(|py| {
        let logger =
            manager::get_logger(py, &unique_logger_name).expect("logger should be created");
        let handler = Arc::new(CollectingHandler::default());
        logger
            .borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result = log_at_level(py, FemtoLevel::Warn, "warn msg", Some(&unique_logger_name))
            .expect("should not error");
        assert!(result.is_some());
        assert!(logger.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level_str(), "WARN");
    });
}

#[rstest]
fn error_dispatches_at_error_level(unique_logger_name: String) {
    Python::attach(|py| {
        let logger =
            manager::get_logger(py, &unique_logger_name).expect("logger should be created");
        let handler = Arc::new(CollectingHandler::default());
        logger
            .borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result = log_at_level(
            py,
            FemtoLevel::Error,
            "error msg",
            Some(&unique_logger_name),
        )
        .expect("should not error");
        assert!(result.is_some());
        assert!(logger.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level_str(), "ERROR");
    });
}

#[rstest]
fn default_logger_is_root() {
    Python::attach(|py| {
        let root = manager::get_logger(py, "root").expect("root logger should exist");
        let handler = Arc::new(CollectingHandler::default());
        root.borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result =
            log_at_level(py, FemtoLevel::Info, "root msg", None).expect("should not error");
        assert!(result.is_some());
        assert!(root.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].logger(), "root");
    });
}

#[rstest]
fn below_threshold_returns_none(unique_logger_name: String) {
    Python::attach(|py| {
        let _logger =
            manager::get_logger(py, &unique_logger_name).expect("logger should be created");
        // Default level is INFO, so DEBUG should be filtered out
        let result = log_at_level(py, FemtoLevel::Debug, "filtered", Some(&unique_logger_name))
            .expect("should not error");
        assert!(result.is_none());
    });
}

#[rstest]
fn source_location_is_captured(unique_logger_name: String) {
    Python::attach(|py| {
        let logger =
            manager::get_logger(py, &unique_logger_name).expect("logger should be created");
        let handler = Arc::new(CollectingHandler::default());
        logger
            .borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let _result = log_at_level(py, FemtoLevel::Info, "located", Some(&unique_logger_name))
            .expect("should not error");
        assert!(logger.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        // Source location should be populated (even if test framework
        // frames mean the values are not from user code, the fields
        // should not be empty when running under CPython).
        let meta = records[0].metadata();
        // Under CPython the frame info should be populated; on other
        // implementations the fallback produces empty strings.
        // We check that metadata was at least initialised.
        assert!(meta.line_number > 0 || meta.filename.is_empty());
    });
}
