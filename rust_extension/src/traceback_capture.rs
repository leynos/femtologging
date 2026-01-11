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
use pyo3::types::{PyBool, PyDict, PyList, PyString, PyTuple};

use crate::exception_schema::{EXCEPTION_SCHEMA_VERSION, ExceptionPayload, StackTracePayload};
use crate::traceback_frames::{
    extract_frames_from_stack_summary, extract_frames_from_tb_exception, get_optional_attr,
};

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
    // Handle exc_info=True: use sys.exc_info()
    if let Ok(b) = exc_info.downcast::<PyBool>() {
        if b.is_true() {
            return capture_from_sys_exc_info(py);
        }
        // exc_info=False means no exception
        return Ok(None);
    }

    // Handle 3-tuple (type, value, traceback)
    if let Ok(tuple) = exc_info.downcast::<PyTuple>()
        && tuple.len() == 3
    {
        let exc_value = tuple.get_item(1)?;
        if exc_value.is_none() {
            return Ok(None);
        }
        return capture_from_exception_tuple(py, tuple);
    }

    // Handle exception instance directly
    if is_exception_instance(py, exc_info)? {
        return capture_from_exception_instance(py, exc_info);
    }

    // Invalid exc_info format
    Err(pyo3::exceptions::PyTypeError::new_err(
        "exc_info must be True, an exception instance, or a 3-tuple (type, value, traceback)",
    ))
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
    let tuple = exc_info.downcast::<PyTuple>()?;

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

/// Build payload from a `TracebackException` object.
///
/// The `exc_value` parameter is the original exception instance. It's optional
/// because for chained exceptions we may not have direct access to the exception
/// instance (only the TracebackException).
fn build_payload_from_traceback_exception(
    py: Python<'_>,
    tb_exc: &Bound<'_, PyAny>,
    exc_value: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<ExceptionPayload>> {
    // Extract exception type name
    let exc_type = tb_exc.getattr("exc_type")?;
    let type_name: String = exc_type.getattr("__name__")?.extract()?;

    // Extract module (None for builtins)
    let module: Option<String> = exc_type
        .getattr("__module__")
        .ok()
        .and_then(|m| m.extract().ok())
        .filter(|m: &String| m != "builtins");

    // Extract message
    let message = format_exception_message(tb_exc)?;

    // Extract args_repr from original exception
    let args_repr = if let Some(exc) = exc_value {
        extract_args_repr_from_exc(exc)?
    } else {
        Vec::new()
    };

    // Extract notes from original exception (__notes__ attribute, Python 3.11+)
    let notes = if let Some(exc) = exc_value {
        extract_notes_from_exc(exc)?
    } else {
        Vec::new()
    };

    // Extract stack frames
    let frames = extract_frames_from_tb_exception(tb_exc)?;

    // Handle exception chaining
    let cause = extract_chained_exception(py, tb_exc, "__cause__")?;
    let context = extract_chained_exception(py, tb_exc, "__context__")?;

    // Check suppress_context
    let suppress_context: bool = tb_exc
        .getattr("__suppress_context__")
        .and_then(|v| v.extract())
        .unwrap_or(false);

    // Handle ExceptionGroup (Python 3.11+)
    let exceptions = extract_exception_group(py, tb_exc)?;

    Ok(Some(ExceptionPayload {
        schema_version: EXCEPTION_SCHEMA_VERSION,
        type_name,
        module,
        message,
        args_repr,
        notes,
        frames,
        cause: cause.map(Box::new),
        context: context.map(Box::new),
        suppress_context,
        exceptions,
    }))
}

/// Format the exception message from a TracebackException.
fn format_exception_message(tb_exc: &Bound<'_, PyAny>) -> PyResult<String> {
    // _str is the formatted exception message
    let msg = tb_exc.getattr("_str")?;
    if msg.is_none() {
        return Ok(String::new());
    }
    // _str can be a tuple or a string
    if let Ok(tuple) = msg.downcast::<PyTuple>() {
        // For exceptions with multiple args, _str is a tuple
        let parts: Vec<String> = tuple
            .iter()
            .filter_map(|item| item.str().ok())
            .filter_map(|s| s.extract().ok())
            .collect();
        return Ok(parts.join(", "));
    }
    msg.str()?.extract()
}

/// Extract args as string representations from the exception instance.
fn extract_args_repr_from_exc(exc: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    let args = match exc.getattr("args") {
        Ok(a) => a,
        Err(_) => return Ok(Vec::new()),
    };
    if args.is_none() {
        return Ok(Vec::new());
    }

    let args_tuple = match args.downcast::<PyTuple>() {
        Ok(t) => t,
        Err(_) => return Ok(Vec::new()),
    };
    let mut result = Vec::with_capacity(args_tuple.len());
    for arg in args_tuple.iter() {
        result.push(arg.repr()?.extract()?);
    }
    Ok(result)
}

/// Extract exception notes from the exception instance (__notes__).
///
/// Per the ADR "partial extraction of collections" rule, individual elements
/// that are not strings are skipped. Only actual Python `str` objects are
/// included in the result; non-strings are silently ignored.
fn extract_notes_from_exc(exc: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    let Some(notes_list): Option<Bound<'_, PyList>> = get_optional_attr(exc, "__notes__") else {
        return Ok(Vec::new());
    };
    let mut result = Vec::with_capacity(notes_list.len());
    for item in notes_list.iter() {
        // Only include actual string objects; skip non-strings per ADR
        if let Ok(s) = item.downcast::<PyString>()
            && let Ok(extracted) = s.extract::<String>()
        {
            result.push(extracted);
        }
    }
    Ok(result)
}

/// Extract a chained exception (__cause__ or __context__).
fn extract_chained_exception(
    py: Python<'_>,
    tb_exc: &Bound<'_, PyAny>,
    attr_name: &str,
) -> PyResult<Option<ExceptionPayload>> {
    let chained = match tb_exc.getattr(attr_name) {
        Ok(c) if !c.is_none() => c,
        _ => return Ok(None),
    };

    // chained is itself a TracebackException
    // We don't have direct access to the chained exception instance here
    build_payload_from_traceback_exception(py, &chained, None)?
        .map(Ok)
        .transpose()
}

/// Extract nested exceptions from an ExceptionGroup.
fn extract_exception_group(
    py: Python<'_>,
    tb_exc: &Bound<'_, PyAny>,
) -> PyResult<Vec<ExceptionPayload>> {
    // Check if this is an ExceptionGroup by looking for 'exceptions' attribute
    let exceptions_attr = match tb_exc.getattr("exceptions") {
        Ok(e) if !e.is_none() => e,
        _ => return Ok(Vec::new()),
    };

    let exceptions_list = exceptions_attr.downcast::<PyList>()?;
    let mut result = Vec::with_capacity(exceptions_list.len());

    for nested_tb_exc in exceptions_list.iter() {
        // We don't have direct access to the nested exception instances here
        if let Some(payload) = build_payload_from_traceback_exception(py, &nested_tb_exc, None)? {
            result.push(payload);
        }
    }

    Ok(result)
}

/// Check if an object is an exception instance.
fn is_exception_instance(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    let base_exception = py.import("builtins")?.getattr("BaseException")?;
    obj.is_instance(&base_exception)
}
