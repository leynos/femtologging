//! Unit tests for the Python convenience logging functions.

use super::*;
use crate::handler::FemtoHandlerTrait;
use crate::log_context;
use crate::log_record::RecordMetadata;
use crate::logger::{FemtoLogger, context_drop_diagnostic};
use crate::test_utils::collecting_handler::CollectingHandler;
use crate::test_utils::logging_test_helpers::unique_logger_name as next_logger_name;
use pyo3::types::PyDict;
use rstest::rstest;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A fixture logger name that keeps test signatures out of the string-heavy band.
///
/// `CodeScene` measures the share of string arguments per module. The logger
/// name is a value the tests pass around rather than free text, so wrapping it
/// keeps the signatures expressive without tripping that threshold.
struct LoggerName(String);

impl LoggerName {
    /// Borrow the name for the logging APIs, which take `&str`.
    fn as_str(&self) -> &str {
        &self.0
    }
}

#[rstest::fixture]
fn unique_logger_name() -> LoggerName {
    LoggerName(next_logger_name())
}

/// Fetch `name`'s logger and attach a fresh [`CollectingHandler`] to it.
///
/// This cannot be an rstest fixture because the logger is GIL-bound, so it is a
/// plain fallible helper invoked from inside `Python::attach`.
fn logger_with_collecting_handler(
    py: Python<'_>,
    name: &str,
) -> PyResult<(Py<FemtoLogger>, Arc<CollectingHandler>)> {
    let logger = manager::get_logger(py, name)?;
    let handler = Arc::new(CollectingHandler::default());
    logger
        .borrow(py)
        .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>)?;
    Ok((logger, handler))
}

#[rstest]
#[case::debug(FemtoLevel::Debug, "DEBUG", "debug msg", true)]
#[case::info(FemtoLevel::Info, "INFO", "info msg", false)]
#[case::warn(FemtoLevel::Warn, "WARN", "warn msg", false)]
#[case::error(FemtoLevel::Error, "ERROR", "error msg", false)]
fn log_dispatches_at_specified_level(
    unique_logger_name: LoggerName,
    #[case] level: FemtoLevel,
    #[case] expected_level_str: &str,
    #[case] message: &str,
    #[case] set_debug_level: bool,
) {
    Python::attach(|py| {
        let (logger, handler) = logger_with_collecting_handler(py, unique_logger_name.as_str())
            .expect("logger should be created");
        if set_debug_level {
            logger.borrow(py).set_level(FemtoLevel::Debug);
        }

        let result = log_at_level(py, level, message, Some(unique_logger_name.as_str()))
            .expect("should not error");
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
        let (root, handler) =
            logger_with_collecting_handler(py, "root").expect("root logger should exist");

        let result =
            log_at_level(py, FemtoLevel::Info, "root msg", None).expect("should not error");
        assert!(result.is_some());
        assert!(root.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].logger(), "root");

        root.borrow(py)
            .remove_handler(&(handler.clone() as Arc<dyn FemtoHandlerTrait>));
    });
}

#[rstest]
fn below_threshold_returns_none(unique_logger_name: LoggerName) {
    Python::attach(|py| {
        let _logger =
            manager::get_logger(py, unique_logger_name.as_str()).expect("logger should be created");
        // Default level is INFO, so DEBUG should be filtered out
        let result = log_at_level(
            py,
            FemtoLevel::Debug,
            "filtered",
            Some(unique_logger_name.as_str()),
        )
        .expect("should not error");
        assert!(result.is_none());
    });
}

#[rstest]
fn source_location_falls_back_gracefully(unique_logger_name: LoggerName) {
    // When called from a pure-Rust context via `Python::attach`, there are
    // no Python frames on the call stack, so `sys._getframe` cannot
    // retrieve source location.  The fallback should produce empty/zero
    // metadata rather than raising an error.
    Python::attach(|py| {
        let (logger, handler) = logger_with_collecting_handler(py, unique_logger_name.as_str())
            .expect("logger should be created");

        let _result = log_at_level(
            py,
            FemtoLevel::Info,
            "located",
            Some(unique_logger_name.as_str()),
        )
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

#[rstest]
fn module_function_does_not_merge_rust_scoped_context(unique_logger_name: LoggerName) {
    Python::attach(|py| {
        let (logger, handler) = logger_with_collecting_handler(py, unique_logger_name.as_str())
            .expect("logger should be created");
        let _guard = log_context::push_log_context([("request_id", "rust-request")])
            .expect("context push should succeed");

        let result = log_at_level(
            py,
            FemtoLevel::Info,
            "module function",
            Some(unique_logger_name.as_str()),
        )
        .expect("module logging should not fail");
        assert!(result.is_some());
        assert!(logger.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        assert!(!records[0].metadata().key_values.contains_key("request_id"));
    });
}

#[rstest]
fn context_rejects_invalid_value_type() {
    Python::attach(|py| {
        let ctx = PyDict::new(py);
        ctx.set_item("bad", PyDict::new(py))
            .expect("set nested dict value");
        let err = py_validate_log_context(&ctx).expect_err("nested dict should be rejected");
        assert!(
            err.to_string()
                .contains("context values must be str, int, float, bool, or None"),
            "unexpected error: {err}"
        );
    });
}

#[rstest]
fn invalid_merged_context_drops_record(unique_logger_name: LoggerName) {
    Python::attach(|py| {
        let (logger, handler) = logger_with_collecting_handler(py, unique_logger_name.as_str())
            .expect("logger should be created");

        let mut invalid_key_values = BTreeMap::new();
        invalid_key_values.insert(String::from("oversize"), "x".repeat(1_025));
        let metadata = RecordMetadata {
            key_values: invalid_key_values,
            ..Default::default()
        };
        let result = logger
            .borrow(py)
            .log_with_metadata(FemtoLevel::Info, "should drop", metadata);

        assert!(
            result.is_none(),
            "invalid merged context should drop record"
        );
        assert!(logger.borrow(py).flush_handlers());
        assert!(
            handler.collected().is_empty(),
            "no records should be emitted when context merge fails"
        );
        assert_eq!(logger.borrow(py).get_context_dropped(), 1);
        assert_eq!(logger.borrow(py).get_dropped(), 0);

        log_context::clear_log_context_for_test();
    });
}

#[rstest]
fn invalid_merged_context_is_dropped_and_counted(unique_logger_name: LoggerName) {
    Python::attach(|py| {
        let (logger, handler) = logger_with_collecting_handler(py, unique_logger_name.as_str())
            .expect("logger should be created");

        let mut invalid_key_values = BTreeMap::new();
        invalid_key_values.insert(String::from("oversize"), "x".repeat(1_025));
        let metadata = RecordMetadata {
            key_values: invalid_key_values.clone(),
            ..Default::default()
        };
        let result = logger
            .borrow(py)
            .log_with_metadata(FemtoLevel::Info, "should drop", metadata);

        assert!(
            result.is_none(),
            "invalid merged context should drop record"
        );
        // The diagnostic goes to the process's stderr descriptor, which
        // redirecting Python's `sys.stderr` cannot observe. The count proves
        // the reporting arm ran; `report_context_drop_tests` pins the exact
        // bytes it writes through an injected writer.
        assert_eq!(
            logger.borrow(py).get_context_dropped(),
            1,
            "the invalid context must be counted as a drop",
        );
        let error = log_context::merge_context_values(&invalid_key_values)
            .expect_err("the oversize value must be refused");
        let diagnostic = context_drop_diagnostic(&error);
        assert!(
            diagnostic.contains("dropping record due to invalid context payload"),
            "the drop diagnostic must name the fault, got {diagnostic:?}",
        );
        assert_eq!(logger.borrow(py).get_dropped(), 0);
        assert!(logger.borrow(py).flush_handlers());
        assert!(
            handler.collected().is_empty(),
            "no records should be emitted when context merge fails"
        );

        log_context::clear_log_context_for_test();
    });
}
