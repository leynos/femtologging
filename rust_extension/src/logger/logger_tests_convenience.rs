//! Tests for convenience logging methods and `isEnabledFor`.

use super::*;
use pyo3::Python;
use pyo3::types::PyBool;
use rstest::rstest;

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
        let result = match method_name {
            "py_debug" => logger.py_debug(py, "hello", None, None),
            "py_info" => logger.py_info(py, "hello", None, None),
            "py_warning" => logger.py_warning(py, "hello", None, None),
            "py_error" => logger.py_error(py, "hello", None, None),
            "py_critical" => logger.py_critical(py, "hello", None, None),
            _ => unreachable!(),
        }
        .expect("convenience method should not fail");
        assert_eq!(result, Some(expected.to_string()));
    });
}

#[test]
fn convenience_methods_respect_level_filtering() {
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        logger.set_level(FemtoLevel::Error);

        assert!(
            logger
                .py_debug(py, "filtered", None, None)
                .expect("should not fail")
                .is_none()
        );
        assert!(
            logger
                .py_info(py, "filtered", None, None)
                .expect("should not fail")
                .is_none()
        );
        assert!(
            logger
                .py_warning(py, "filtered", None, None)
                .expect("should not fail")
                .is_none()
        );
        assert!(
            logger
                .py_error(py, "emitted", None, None)
                .expect("should not fail")
                .is_some()
        );
        assert!(
            logger
                .py_critical(py, "emitted", None, None)
                .expect("should not fail")
                .is_some()
        );
    });
}

#[test]
fn exception_without_exc_info_passes_true() {
    // Verify that exception() with no explicit exc_info argument
    // delegates to py_log with exc_info=True (a boolean True value).
    // When no active exception exists, exc_info=True is a no-op,
    // so the output is the plain formatted message at ERROR level.
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let result = logger
            .py_exception(py, "no active exc", None, None)
            .expect("exception() with no active exception should not fail");
        assert_eq!(result, Some("test [ERROR] no active exc".to_string()));
    });
}

#[test]
fn exception_with_explicit_exc_info_false() {
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let false_val = PyBool::new(py, false).to_owned().into_any();
        let result = logger
            .py_exception(py, "no capture", Some(&false_val.as_borrowed()), None)
            .expect("exception() with exc_info=False should not fail");
        assert_eq!(result, Some("test [ERROR] no capture".to_string()));
    });
}
