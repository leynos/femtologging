//! Capture-counter tests for deferred traceback capture.
//!
//! The ordering tests prove *when* capture runs. These tests prove *whether*
//! it ran at all, using the test-only count of real traversals so that a
//! suppressed capture is distinguishable from a capture whose payload
//! happened to be small.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;

use super::FemtoLogger;
use crate::filters::{FilterBuilderTrait, LevelFilterBuilder};
use crate::level::FemtoLevel;
use crate::traceback_capture::{capture_count, reset_capture_count};

/// Build a logger whose level admits `FemtoLevel::Info` records.
fn info_logger(name: &str) -> FemtoLogger {
    let logger = FemtoLogger::new(name.to_owned()).expect("test logger should be constructible");
    logger.set_level(FemtoLevel::Trace);
    logger
}

/// Build a real Python exception instance for capture-counter tests.
fn python_exception(py: Python<'_>) -> pyo3::Bound<'_, pyo3::PyAny> {
    py.import("builtins")
        .expect("builtins should import")
        .getattr("ValueError")
        .expect("ValueError should exist")
        .call1(("boom",))
        .expect("ValueError should construct")
}

#[test]
fn py_log_below_threshold_performs_no_capture() {
    Python::attach(|py| {
        let logger = FemtoLogger::new("deferred-py-threshold".to_owned())
            .expect("test logger should be constructible");
        logger.set_level(FemtoLevel::Error);
        let exception = python_exception(py);
        reset_capture_count();

        let result = logger
            .py_log(
                py,
                FemtoLevel::Info,
                "ignored",
                Some(&exception),
                Some(true),
                None,
            )
            .expect("a below-threshold record must not raise");

        assert!(result.is_none());
        assert_eq!(
            capture_count(),
            0,
            "no traversal may run below the logger threshold"
        );
    });
}

#[test]
fn py_log_native_filter_rejection_performs_no_capture() {
    Python::attach(|py| {
        let logger = info_logger("deferred-py-filter");
        let rejecting = LevelFilterBuilder::new()
            .with_max_level(FemtoLevel::Trace)
            .build()
            .expect("level filter should build");
        logger.add_filter(rejecting);
        let exception = python_exception(py);
        reset_capture_count();

        let result = logger
            .py_log(
                py,
                FemtoLevel::Info,
                "ignored",
                Some(&exception),
                Some(true),
                None,
            )
            .expect("filter rejection must not raise");

        assert!(result.is_none());
        assert_eq!(
            capture_count(),
            0,
            "no traversal may run for a record the native prefix rejected"
        );
    });
}

#[test]
fn py_log_invalid_exc_info_raises_even_when_filters_reject() {
    Python::attach(|py| {
        let logger = info_logger("deferred-py-invalid");
        let rejecting = LevelFilterBuilder::new()
            .with_max_level(FemtoLevel::Trace)
            .build()
            .expect("level filter should build");
        logger.add_filter(rejecting);
        let invalid = 42_i64
            .into_pyobject(py)
            .expect("integer is valid")
            .into_any();

        let error = logger
            .py_log(py, FemtoLevel::Info, "invalid", Some(&invalid), None, None)
            .expect_err("an invalid exc_info must raise");

        assert!(
            error.is_instance_of::<PyTypeError>(py),
            "expected TypeError, got {error}"
        );
    });
}

#[test]
fn py_log_accepted_record_captures_each_payload_once() {
    Python::attach(|py| {
        let logger = info_logger("deferred-py-capture");
        let exception = python_exception(py);
        reset_capture_count();

        let result = logger
            .py_log(
                py,
                FemtoLevel::Info,
                "captured",
                Some(&exception),
                Some(true),
                None,
            )
            .expect("capture should not raise");

        assert!(result.is_some());
        assert_eq!(
            capture_count(),
            2,
            "one exception traversal and one stack traversal should run"
        );
    });
}
