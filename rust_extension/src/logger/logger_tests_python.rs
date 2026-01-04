//! Unit tests for Python integration paths in FemtoLogger.
//!
//! These tests require the `python` feature and exercise the PyO3 bindings.

use super::*;
use pyo3::Python;
use pyo3::types::{PyBool, PyTuple};
use rstest::rstest;

// --------------------------------
// Test helpers
// --------------------------------

/// Create a Python exception instance by type name and message.
fn create_py_exception<'py>(
    py: Python<'py>,
    exc_type: &str,
    message: &str,
) -> pyo3::Bound<'py, pyo3::PyAny> {
    py.import("builtins")
        .expect("builtins module should exist")
        .getattr(exc_type)
        .unwrap_or_else(|_| panic!("{exc_type} should exist"))
        .call1((message,))
        .unwrap_or_else(|_| panic!("{exc_type} constructor should succeed"))
}

/// Assert that output contains the base log message and all expected substrings.
fn assert_output_contains(output: Option<String>, expected_substrings: &[&str]) {
    let text = output.expect("Should produce output");
    for substring in expected_substrings {
        assert!(
            text.contains(substring),
            "Output should contain '{substring}', got: {text}"
        );
    }
}

/// Test inputs for `should_capture_exc_info` parameterised testing.
#[derive(Debug)]
enum ExcInfoInput {
    True,
    False,
    None,
    ExceptionInstance,
    Tuple3,
    Integer,
}

/// Expected result from `should_capture_exc_info`.
#[derive(Debug, PartialEq)]
enum ExpectedCapture {
    Capture,
    NoCapture,
}

#[rstest]
#[case(
    ExcInfoInput::True,
    ExpectedCapture::Capture,
    "True should trigger capture"
)]
#[case(
    ExcInfoInput::False,
    ExpectedCapture::NoCapture,
    "False should not trigger capture"
)]
#[case(
    ExcInfoInput::None,
    ExpectedCapture::NoCapture,
    "None should not trigger capture"
)]
#[case(
    ExcInfoInput::ExceptionInstance,
    ExpectedCapture::Capture,
    "Exception instance should trigger capture"
)]
#[case(
    ExcInfoInput::Tuple3,
    ExpectedCapture::Capture,
    "3-tuple should trigger capture"
)]
#[case(
    ExcInfoInput::Integer,
    ExpectedCapture::Capture,
    "Non-None non-False values should trigger capture"
)]
fn should_capture_exc_info_cases(
    #[case] input: ExcInfoInput,
    #[case] expected: ExpectedCapture,
    #[case] description: &str,
) {
    Python::with_gil(|py| {
        let result = match input {
            ExcInfoInput::True => {
                let true_val = PyBool::new(py, true);
                should_capture_exc_info(true_val.as_any())
            }
            ExcInfoInput::False => {
                let false_val = PyBool::new(py, false);
                should_capture_exc_info(false_val.as_any())
            }
            ExcInfoInput::None => {
                let none = py.None();
                should_capture_exc_info(none.bind(py))
            }
            ExcInfoInput::ExceptionInstance => {
                let exc = create_py_exception(py, "ValueError", "test error");
                should_capture_exc_info(&exc)
            }
            ExcInfoInput::Tuple3 => {
                let exc_value = create_py_exception(py, "KeyError", "key");
                let exc_type = exc_value.get_type();
                let exc_tb = py.None();
                let tuple = PyTuple::new(
                    py,
                    &[exc_type.as_any(), exc_value.as_any(), exc_tb.bind(py)],
                )
                .expect("tuple creation should succeed");
                should_capture_exc_info(tuple.as_any())
            }
            ExcInfoInput::Integer => {
                let code = c"42";
                let int_val = py
                    .eval(code, None, None)
                    .expect("eval of integer should succeed");
                should_capture_exc_info(&int_val)
            }
        };

        let capture_result = result.expect("should_capture_exc_info should not fail");
        let expected_bool = expected == ExpectedCapture::Capture;
        assert_eq!(capture_result, expected_bool, "{description}");
    });
}

// --------------------------------
// Tests for py_log
// --------------------------------

#[test]
fn py_log_basic_message() {
    Python::with_gil(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let result = logger
            .py_log(py, FemtoLevel::Info, "hello", None, None)
            .expect("py_log should not fail");
        assert_eq!(result, Some("test [INFO] hello".to_string()));
    });
}

#[test]
fn py_log_filtered_by_level() {
    Python::with_gil(|py| {
        let logger = FemtoLogger::new("test".to_string());
        logger.set_level(FemtoLevel::Error);
        let result = logger
            .py_log(py, FemtoLevel::Info, "ignored", None, None)
            .expect("py_log should not fail");
        assert!(
            result.is_none(),
            "Message below level threshold should be filtered"
        );
    });
}

#[test]
fn py_log_with_exc_info_false() {
    Python::with_gil(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let false_val = PyBool::new(py, false);
        let result = logger
            .py_log(
                py,
                FemtoLevel::Error,
                "no traceback",
                Some(false_val.as_any()),
                None,
            )
            .expect("py_log should not fail with exc_info=False");
        assert_eq!(result, Some("test [ERROR] no traceback".to_string()));
    });
}

#[test]
fn py_log_with_exc_info_none() {
    Python::with_gil(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let none = py.None();
        let result = logger
            .py_log(
                py,
                FemtoLevel::Error,
                "no traceback",
                Some(none.bind(py)),
                None,
            )
            .expect("py_log should not fail with exc_info=None");
        assert_eq!(result, Some("test [ERROR] no traceback".to_string()));
    });
}

#[test]
fn py_log_with_stack_info_false() {
    Python::with_gil(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let result = logger
            .py_log(py, FemtoLevel::Info, "no stack", None, Some(false))
            .expect("py_log should not fail with stack_info=false");
        assert_eq!(result, Some("test [INFO] no stack".to_string()));
    });
}

#[test]
fn py_log_with_stack_info_true() {
    Python::with_gil(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let result = logger
            .py_log(py, FemtoLevel::Info, "with stack", None, Some(true))
            .expect("py_log should not fail with stack_info=true");

        assert_output_contains(result, &["test [INFO] with stack", "Stack"]);
    });
}

#[test]
fn py_log_with_exception_instance() {
    Python::with_gil(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let exc = create_py_exception(py, "ValueError", "test error");

        let result = logger
            .py_log(py, FemtoLevel::Error, "caught", Some(&exc), None)
            .expect("py_log should not fail with exception instance");

        assert_output_contains(result, &["test [ERROR] caught", "ValueError", "test error"]);
    });
}

#[test]
fn py_log_with_exc_info_and_stack_info() {
    Python::with_gil(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let exc = create_py_exception(py, "ValueError", "combined test");

        let result = logger
            .py_log(py, FemtoLevel::Error, "both", Some(&exc), Some(true))
            .expect("py_log should not fail with both exc_info and stack_info");

        assert_output_contains(
            result,
            &["test [ERROR] both", "ValueError", "combined test", "Stack"],
        );
    });
}
