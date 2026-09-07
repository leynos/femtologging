//! Unit tests for the Python convenience logging functions.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use pyo3::types::PyDict;
use rstest::{fixture, rstest};

use super::*;
use crate::{
    handler::FemtoHandlerTrait,
    log_context,
    log_record::RecordMetadata,
    logger::FemtoLogger,
    test_utils::collecting_handler::CollectingHandler,
};

static LOGGER_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
struct LogDispatchCase {
    level: FemtoLevel,
    expected_level_str: &'static str,
    message: &'static str,
    set_debug_level: bool,
}

#[fixture]
fn unique_logger_name() -> String {
    let suffix = LOGGER_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("conv.test.{suffix}")
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
        .add_handler(handler.clone() as Arc<dyn FemtoHandlerTrait>);
    Ok((logger, handler))
}

#[rstest]
#[case::debug(LogDispatchCase { level: FemtoLevel::Debug, expected_level_str: "DEBUG", message: "debug msg", set_debug_level: true })]
#[case::info(LogDispatchCase { level: FemtoLevel::Info, expected_level_str: "INFO", message: "info msg", set_debug_level: false })]
#[case::warn(LogDispatchCase { level: FemtoLevel::Warn, expected_level_str: "WARN", message: "warn msg", set_debug_level: false })]
#[case::error(LogDispatchCase { level: FemtoLevel::Error, expected_level_str: "ERROR", message: "error msg", set_debug_level: false })]
fn log_dispatches_at_specified_level(unique_logger_name: String, #[case] case: LogDispatchCase) {
    Python::attach(|py| {
        let (logger, handler) = logger_with_collecting_handler(py, &unique_logger_name)
            .expect("logger should be created");
        if case.set_debug_level {
            logger.borrow(py).set_level(FemtoLevel::Debug);
        }

        let result = log_at_level(py, case.level, case.message, Some(&unique_logger_name))
            .expect("should not error");
        assert!(result.is_some());
        assert!(logger.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        let record = records.first().expect("one record should be collected");
        assert_eq!(record.level_str(), case.expected_level_str);
        assert_eq!(record.message(), case.message);
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
        let record = records.first().expect("one record should be collected");
        assert_eq!(record.logger(), "root");

        root.borrow(py)
            .remove_handler(&(handler.clone() as Arc<dyn FemtoHandlerTrait>));
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
        let (logger, handler) = logger_with_collecting_handler(py, &unique_logger_name)
            .expect("logger should be created");

        let _result = log_at_level(py, FemtoLevel::Info, "located", Some(&unique_logger_name))
            .expect("should not error");
        assert!(logger.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        let record = records.first().expect("one record should be collected");
        let meta = record.metadata();
        assert_eq!(meta.filename, "", "fallback filename should be empty");
        assert_eq!(meta.line_number, 0, "fallback line number should be zero");
        assert_eq!(meta.module_path, "", "fallback module_path should be empty");
    });
}

#[rstest]
fn scoped_context_is_attached_to_convenience_logs(unique_logger_name: String) {
    struct LogContextPopGuard;

    impl Drop for LogContextPopGuard {
        fn drop(&mut self) { drop(py_pop_log_context()); }
    }

    Python::attach(|py| {
        let (logger, handler) = logger_with_collecting_handler(py, &unique_logger_name)
            .expect("logger should be created");

        let ctx = PyDict::new(py);
        ctx.set_item("request_id", 42).expect("set request_id");
        ctx.set_item("user", "alice").expect("set user");
        py_push_log_context(&ctx).expect("context push should succeed");
        let _guard = LogContextPopGuard;
        let result = log_at_level(
            py,
            FemtoLevel::Info,
            "with context",
            Some(&unique_logger_name),
        )
        .expect("log call should succeed");
        assert!(result.is_some());
        assert!(logger.borrow(py).flush_handlers());

        let records = handler.collected();
        assert_eq!(records.len(), 1);
        let record = records.first().expect("one record should be collected");
        let key_values = &record.metadata().key_values;
        assert_eq!(key_values.get("request_id").map(String::as_str), Some("42"));
        assert_eq!(key_values.get("user").map(String::as_str), Some("alice"));
    });
}

#[rstest]
fn context_rejects_invalid_value_type() {
    Python::attach(|py| {
        let ctx = PyDict::new(py);
        ctx.set_item("bad", PyDict::new(py))
            .expect("set nested dict value");
        let err = py_push_log_context(&ctx).expect_err("nested dict should be rejected");
        assert!(
            err.to_string()
                .contains("context values must be str, int, float, bool, or None"),
            "unexpected error: {err}"
        );
        log_context::clear_log_context_for_test();
    });
}

#[rstest]
fn invalid_merged_context_drops_record(unique_logger_name: String) {
    Python::attach(|py| {
        let (logger, handler) = logger_with_collecting_handler(py, &unique_logger_name)
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

        log_context::clear_log_context_for_test();
    });
}
