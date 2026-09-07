//! Unit tests for Python integration paths in FemtoLogger.
//!
//! These tests require the `python` feature and exercise the PyO3 bindings.

use super::*;
use pyo3::Python;
use pyo3::types::{PyBool, PyDict, PyTuple};
use rstest::rstest;

// --------------------------------
// Test helpers
// --------------------------------

/// Create a Python exception instance by type name and message.
fn create_py_exception<'py>(
    py: Python<'py>,
    exc_type: &str,
    message: &str,
) -> PyResult<pyo3::Bound<'py, pyo3::PyAny>> {
    py.import("builtins")?.getattr(exc_type)?.call1((message,))
}

/// Construct positional logging arguments, propagating Python errors.
fn log_args<'py>(
    py: Python<'py>,
    level: &str,
    message: &str,
) -> PyResult<pyo3::Bound<'py, PyTuple>> {
    PyTuple::new(py, [level, message])
}

/// Assert that output contains the base log message and all expected substrings.
///
/// A macro rather than a function so the panic points at the calling test
/// and the expect lint sees the unwrap inside a recognized test body.
macro_rules! assert_output_contains {
    ($output:expr, $expected_substrings:expr) => {{
        let text = $output.expect("Should produce output");
        for substring in $expected_substrings {
            assert!(
                text.contains(substring),
                "Output should contain '{substring}', got: {text}"
            );
        }
    }};
}

/// Test inputs for `should_capture_exc_info` parameterized testing.
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

/// Test inputs for py_log exc_info parameter tests.
#[derive(Debug)]
enum PyLogExcInfoInput {
    BoolFalse,
    PythonNone,
    ExceptionInstance {
        exc_type: &'static str,
        exc_msg: &'static str,
    },
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
    Python::attach(|py| {
        let capture_result = match input {
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
                let exc = create_py_exception(py, "ValueError", "test error")
                    .expect("exception construction should succeed");
                should_capture_exc_info(&exc)
            }
            ExcInfoInput::Tuple3 => {
                let exc_value = create_py_exception(py, "KeyError", "key")
                    .expect("exception construction should succeed");
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

        let expected_bool = expected == ExpectedCapture::Capture;
        assert_eq!(capture_result, expected_bool, "{description}");
    });
}

// --------------------------------
// Tests for py_log
// --------------------------------

#[test]
fn py_log_basic_message() {
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let args =
            log_args(py, "INFO", "hello").expect("Python log arguments should be constructible");
        let result = logger
            .py_log(py, &args, None)
            .expect("py_log should not fail");
        assert_eq!(result, Some("test [INFO] hello".to_string()));
    });
}

#[test]
fn py_log_filtered_by_level() {
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        logger.set_level(FemtoLevel::Error);
        let args =
            log_args(py, "INFO", "ignored").expect("Python log arguments should be constructible");
        let result = logger
            .py_log(py, &args, None)
            .expect("py_log should not fail");
        assert!(
            result.is_none(),
            "Message below level threshold should be filtered"
        );
    });
}

#[rstest]
#[case::exc_info_false(
    PyLogExcInfoInput::BoolFalse,
    "no traceback",
    None,
    "test [ERROR] no traceback"
)]
#[case::exc_info_none(
    PyLogExcInfoInput::PythonNone,
    "no traceback",
    None,
    "test [ERROR] no traceback"
)]
#[case::exception_instance(
    PyLogExcInfoInput::ExceptionInstance { exc_type: "ValueError", exc_msg: "test error" },
    "caught",
    None,
    "test [ERROR] caught\nValueError\ntest error"
)]
#[case::exc_info_and_stack_info(
    PyLogExcInfoInput::ExceptionInstance { exc_type: "ValueError", exc_msg: "combined test" },
    "both",
    Some(true),
    "test [ERROR] both\nValueError\ncombined test\nStack"
)]
fn py_log_exc_info_variation_cases(
    #[case] input: PyLogExcInfoInput,
    #[case] message: &str,
    #[case] stack_info: Option<bool>,
    #[case] expected: &str,
) {
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let exc_info: Option<pyo3::Bound<'_, pyo3::PyAny>> = match &input {
            PyLogExcInfoInput::BoolFalse => Some(PyBool::new(py, false).to_owned().into_any()),
            PyLogExcInfoInput::PythonNone => Some(py.None().into_bound(py)),
            PyLogExcInfoInput::ExceptionInstance { exc_type, exc_msg } => Some(
                create_py_exception(py, exc_type, exc_msg)
                    .expect("exception construction should succeed"),
            ),
        };

        let args =
            log_args(py, "ERROR", message).expect("Python log arguments should be constructible");
        let keyword_args = PyDict::new(py);
        if let Some(exc_info) = exc_info {
            keyword_args
                .set_item("exc_info", exc_info)
                .expect("keyword argument insertion should succeed");
        }
        if let Some(stack_info) = stack_info {
            keyword_args
                .set_item("stack_info", stack_info)
                .expect("keyword argument insertion should succeed");
        }
        let result = logger
            .py_log(py, &args, Some(&keyword_args))
            .expect("py_log should succeed");

        // expected contains newline-separated substrings to check
        let expected_parts: Vec<&str> = expected.split('\n').collect();
        if expected_parts.len() == 1 {
            assert_eq!(result, Some(expected.to_string()));
        } else {
            assert_output_contains!(result, &expected_parts);
        }
    });
}

#[test]
fn py_log_with_stack_info_false() {
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let args =
            log_args(py, "INFO", "no stack").expect("Python log arguments should be constructible");
        let keyword_args = PyDict::new(py);
        keyword_args
            .set_item("stack_info", false)
            .expect("keyword argument insertion should succeed");
        let result = logger
            .py_log(py, &args, Some(&keyword_args))
            .expect("py_log should not fail with stack_info=false");
        assert_eq!(result, Some("test [INFO] no stack".to_string()));
    });
}

#[test]
fn py_log_with_stack_info_true() {
    Python::attach(|py| {
        let logger = FemtoLogger::new("test".to_string());
        let args = log_args(py, "INFO", "with stack")
            .expect("Python log arguments should be constructible");
        let keyword_args = PyDict::new(py);
        keyword_args
            .set_item("stack_info", true)
            .expect("keyword argument insertion should succeed");
        let result = logger
            .py_log(py, &args, Some(&keyword_args))
            .expect("py_log should not fail with stack_info=true");

        assert_output_contains!(result, &["test [INFO] with stack", "Stack"]);
    });
}
