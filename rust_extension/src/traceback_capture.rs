//! Python traceback capture utilities.
//!
//! This module provides functions to extract exception and stack trace data
//! from Python and convert them to the Rust schema types defined in
//! [`crate::exception_schema`]. All capture happens on the caller thread
//! while the GIL is held, ensuring worker threads remain GIL-free.
//!
//! # Usage
//!
//! The primary entry points are [`capture_exception`] for `exc_info` handling
//! and [`capture_stack`] for `stack_info=True` support.

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyTuple};

use crate::exception_schema::{EXCEPTION_SCHEMA_VERSION, ExceptionPayload, StackTracePayload};
use crate::traceback_frames::extract_frames_from_stack_summary;

mod traceback_payload;
use self::traceback_payload::build_payload_from_traceback_exception;

enum ExcInfoKind {
    BoolTrue,
    BoolFalse,
    Tuple(Py<PyTuple>),
    Exception,
}

fn is_py_bool_true(exc_info: &Bound<'_, PyAny>) -> bool {
    exc_info
        .cast::<PyBool>()
        .is_ok_and(|bool_value| bool_value.is_true())
}

fn is_py_bool_false(exc_info: &Bound<'_, PyAny>) -> bool {
    exc_info
        .cast::<PyBool>()
        .is_ok_and(|bool_value| !bool_value.is_true())
}

fn extract_exc_tuple(exc_info: &Bound<'_, PyAny>) -> Option<Py<PyTuple>> {
    let tuple = exc_info.cast::<PyTuple>().ok()?;
    if tuple.len() == 3 {
        Some(tuple.clone().unbind())
    } else {
        None
    }
}

fn classify_exc_info(py: Python<'_>, exc_info: &Bound<'_, PyAny>) -> PyResult<ExcInfoKind> {
    if is_py_bool_true(exc_info) {
        return Ok(ExcInfoKind::BoolTrue);
    }

    if is_py_bool_false(exc_info) {
        return Ok(ExcInfoKind::BoolFalse);
    }

    if let Some(tuple) = extract_exc_tuple(exc_info) {
        return Ok(ExcInfoKind::Tuple(tuple));
    }

    if is_exception_instance(py, exc_info)? {
        return Ok(ExcInfoKind::Exception);
    }

    Err(pyo3::exceptions::PyTypeError::new_err(
        "exc_info must be True, an exception instance, or a 3-tuple (type, value, traceback)",
    ))
}

/// Capture exception information from Python `exc_info` argument.
///
/// Handles the various forms of `exc_info` accepted by Python's logging:
/// - `True`: Use `sys.exc_info()` to get the current exception
/// - Exception instance: Wrap in a traceback context
/// - 3-tuple `(type, value, traceback)`: Use directly
///
/// Returns `None` if `exc_info=True` but no exception is active.
///
/// # Errors
///
/// Returns an error if Python calls fail or the exc_info format is invalid.
pub fn capture_exception(
    py: Python<'_>,
    exc_info: &Bound<'_, PyAny>,
) -> PyResult<Option<ExceptionPayload>> {
    match classify_exc_info(py, exc_info)? {
        ExcInfoKind::BoolTrue => capture_from_sys_exc_info(py),
        ExcInfoKind::BoolFalse => Ok(None),
        ExcInfoKind::Tuple(tuple) => capture_from_exception_tuple(py, tuple.bind(py)),
        ExcInfoKind::Exception => capture_from_exception_instance(py, exc_info),
    }
}

/// Capture the current call stack for `stack_info=True`.
///
/// Uses `traceback.extract_stack()` to get the current stack frames.
/// The full call stack is returned; frame filtering (e.g., to remove
/// logging infrastructure frames) is left to the caller or formatter.
///
/// # Errors
///
/// Returns an error if Python traceback calls fail.
pub fn capture_stack(py: Python<'_>) -> PyResult<StackTracePayload> {
    let traceback = py.import("traceback")?;
    // extract_stack() returns a StackSummary (list of FrameSummary)
    let stack_summary = traceback.call_method0("extract_stack")?;
    let frames = extract_frames_from_stack_summary(&stack_summary)?;

    Ok(StackTracePayload {
        schema_version: EXCEPTION_SCHEMA_VERSION,
        frames,
    })
}

/// Capture exception from `sys.exc_info()`.
fn capture_from_sys_exc_info(py: Python<'_>) -> PyResult<Option<ExceptionPayload>> {
    let sys = py.import("sys")?;
    let exc_info = sys.call_method0("exc_info")?;
    let tuple = exc_info.cast::<PyTuple>()?;

    // exc_info returns (type, value, traceback), all None if no exception
    let exc_value = tuple.get_item(1)?;
    if exc_value.is_none() {
        return Ok(None);
    }

    capture_from_exception_tuple(py, tuple)
}

/// Capture exception from a 3-tuple (type, value, traceback).
fn capture_from_exception_tuple(
    py: Python<'_>,
    tuple: &Bound<'_, PyTuple>,
) -> PyResult<Option<ExceptionPayload>> {
    let exc_value = tuple.get_item(1)?;
    if exc_value.is_none() {
        return Ok(None);
    }

    let exc_tb = tuple.get_item(2)?;
    let tb_arg = if exc_tb.is_none() {
        None
    } else {
        Some(&exc_tb)
    };

    build_exception_payload(py, &exc_value, tb_arg)
}

/// Capture exception from an exception instance.
fn capture_from_exception_instance(
    py: Python<'_>,
    exc: &Bound<'_, PyAny>,
) -> PyResult<Option<ExceptionPayload>> {
    // Get the traceback from __traceback__ attribute
    let tb = exc.getattr("__traceback__").ok();
    let tb_ref = tb.as_ref().filter(|t| !t.is_none());

    build_exception_payload(py, exc, tb_ref)
}

/// Build an `ExceptionPayload` from a Python exception value and optional traceback.
///
/// When `traceback` is provided, it takes precedence over `exc_value.__traceback__`.
/// This is important for exc_info tuples where the exception's `__traceback__` may
/// have been cleared but a valid traceback was passed explicitly.
fn build_exception_payload(
    py: Python<'_>,
    exc_value: &Bound<'_, PyAny>,
    traceback: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<ExceptionPayload>> {
    let traceback_mod = py.import("traceback")?;
    let tb_exc_class = traceback_mod.getattr("TracebackException")?;
    let kwargs = PyDict::new(py);
    kwargs.set_item("capture_locals", false)?;

    // When we have an explicit traceback (e.g., from exc_info tuple), use the
    // TracebackException constructor directly to preserve it. Otherwise, use
    // from_exception which reads exc_value.__traceback__.
    let tb_exc = if let Some(tb) = traceback {
        // Get the exception type from the value
        let exc_type = exc_value.get_type();
        // TracebackException(exc_type, exc_value, exc_traceback, capture_locals=False)
        tb_exc_class.call((exc_type, exc_value, tb), Some(&kwargs))?
    } else {
        // TracebackException.from_exception(exc, capture_locals=False)
        tb_exc_class.call_method("from_exception", (exc_value,), Some(&kwargs))?
    };

    build_payload_from_traceback_exception(py, &tb_exc, Some(exc_value))
}

/// Check if an object is an exception instance.
fn is_exception_instance(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    let base_exception = py.import("builtins")?.getattr("BaseException")?;
    obj.is_instance(&base_exception)
}
