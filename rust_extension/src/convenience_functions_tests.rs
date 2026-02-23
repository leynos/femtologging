//! Unit tests for the Python convenience logging functions.

use super::*;
use crate::handler::FemtoHandlerTrait;
use crate::test_utils::collecting_handler::CollectingHandler;
use rstest::{fixture, rstest};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

static LOGGER_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[fixture]
fn unique_logger_name() -> String {
    let suffix = LOGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("conv.test.{suffix}")
}

#[rstest]
#[case::debug(FemtoLevel::Debug, "DEBUG", "debug msg", true)]
#[case::info(FemtoLevel::Info, "INFO", "info msg", false)]
#[case::warn(FemtoLevel::Warn, "WARN", "warn msg", false)]
#[case::error(FemtoLevel::Error, "ERROR", "error msg", false)]
fn log_dispatches_at_specified_level(
    unique_logger_name: String,
    #[case] level: FemtoLevel,
    #[case] expected_level_str: &str,
    #[case] message: &str,
    #[case] set_debug_level: bool,
) {
    Python::attach(|py| {
        let logger =
            manager::get_logger(py, &unique_logger_name).expect("logger should be created");
        if set_debug_level {
            logger.borrow(py).set_level(FemtoLevel::Debug);
        }
        let handler = Arc::new(CollectingHandler::default());
        logger
            .borrow(py)
            .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);

        let result =
            log_at_level(py, level, message, Some(&unique_logger_name)).expect("should not error");
        assert!(result.is_some());
        assert!(logger.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].level_str(), expected_level_str);
        assert_eq!(records[0].message(), message);
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
fn source_location_falls_back_gracefully(unique_logger_name: String) {
    // When called from a pure-Rust context via `Python::attach`, there are
    // no Python frames on the call stack, so `sys._getframe` cannot
    // retrieve source location.  The fallback should produce empty/zero
    // metadata rather than raising an error.
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
        let meta = records[0].metadata();
        assert_eq!(meta.filename, "", "fallback filename should be empty");
        assert_eq!(meta.line_number, 0, "fallback line number should be zero");
        assert_eq!(meta.module_path, "", "fallback module_path should be empty");
    });
}
