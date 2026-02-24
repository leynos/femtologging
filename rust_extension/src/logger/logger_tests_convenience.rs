//! Tests for convenience logging methods and `isEnabledFor`.

use super::*;
use pyo3::Python;
use pyo3::types::PyBool;
use rstest::rstest;

/// Dispatch to the named convenience method on `FemtoLogger`.
///
/// Centralises the method-name-to-function mapping so parameterised
/// tests avoid duplicating the five-way match.
fn call_py_log_method(
    logger: &FemtoLogger,
    py: Python<'_>,
    method_name: &str,
    message: &str,
) -> PyResult<Option<String>> {
    match method_name {
        "py_debug" => logger.py_debug(py, message, None, None),
        "py_info" => logger.py_info(py, message, None, None),
        "py_warning" => logger.py_warning(py, message, None, None),
        "py_error" => logger.py_error(py, message, None, None),
        "py_critical" => logger.py_critical(py, message, None, None),
        _ => unreachable!("unknown convenience method: {method_name}"),
    }
}

#[test]
fn is_enabled_for_respects_logger_level() {
    let logger = FemtoLogger::new("test".to_string());
    logger.set_level(FemtoLevel::Warn);
    assert!(!logger.is_enabled_for(FemtoLevel::Debug));
    assert!(!logger.is_enabled_for(FemtoLevel::Info));
    assert!(logger.is_enabled_for(FemtoLevel::Warn));
    assert!(logger.is_enabled_for(FemtoLevel::Error));
    assert!(logger.is_enabled_for(FemtoLevel::Critical));
}

#[test]
fn py_is_enabled_for_mirrors_internal_method() {
    Python::attach(|_py| {
        let logger = FemtoLogger::new("test".to_string());
        logger.set_level(FemtoLevel::Error);
        assert!(!logger.py_is_enabled_for(FemtoLevel::Info));
        assert!(logger.py_is_enabled_for(FemtoLevel::Error));
        assert!(logger.py_is_enabled_for(FemtoLevel::Critical));
    });
}

#[rstest]
#[case::debug("py_debug", "test [DEBUG] hello")]
#[case::info("py_info", "test [INFO] hello")]
#[case::warning("py_warning", "test [WARN] hello")]
#[case::error("py_error", "test [ERROR] hello")]
#[case::critical("py_critical", "test [CRITICAL] hello")]
fn convenience_method_logs_at_correct_level(#[case] method_name: &str, #[case] expected: &str) {
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        logger.set_level(FemtoLevel::Trace);
        let result = call_py_log_method(&logger, py, method_name, "hello")
            .expect("convenience method should not fail");
        assert_eq!(result, Some(expected.to_string()));
    });
}

#[rstest]
#[case::debug("py_debug", false)]
#[case::info("py_info", false)]
#[case::warning("py_warning", false)]
#[case::error("py_error", true)]
#[case::critical("py_critical", true)]
fn convenience_methods_respect_level_filtering(
    #[case] method_name: &str,
    #[case] should_emit: bool,
) {
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        logger.set_level(FemtoLevel::Error);
        let result =
            call_py_log_method(&logger, py, method_name, "filtered").expect("should not fail");
        assert_eq!(
            result.is_some(),
            should_emit,
            "{method_name} with level=Error should{}emit",
            if should_emit { " " } else { " not " }
        );
    });
}

#[test]
fn exception_impl_logs_at_error_level() {
    // _exception_impl (the Rust entry point) passes exc_info through
    // as-is.  The Python-side wrapper in _compat adds the True default.
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let result = logger
            .py_exception(py, "no active exc", None, None)
            .expect("_exception_impl with no exc_info should not fail");
        assert_eq!(result, Some("test [ERROR] no active exc".to_string()));
    });
}

#[test]
fn exception_impl_with_explicit_exc_info_false() {
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let false_val = PyBool::new(py, false).to_owned().into_any();
        let result = logger
            .py_exception(py, "no capture", Some(&false_val.as_borrowed()), None)
            .expect("_exception_impl with exc_info=False should not fail");
        assert_eq!(result, Some("test [ERROR] no capture".to_string()));
    });
}

#[test]
fn exception_impl_with_explicit_exc_info_none_suppresses_capture() {
    // Passing Python None for exc_info should suppress exception capture.
    // should_capture_exc_info returns false for None.
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let none_val = py.None().into_bound(py).into_any();
        let result = logger
            .py_exception(py, "none passed", Some(&none_val.as_borrowed()), None)
            .expect("_exception_impl with exc_info=None should not fail");
        assert_eq!(result, Some("test [ERROR] none passed".to_string()));
    });
}
