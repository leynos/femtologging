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
use pyo3::types::{PyBool, PyDict, PyList, PyTuple};
use std::collections::BTreeMap;

use crate::exception_schema::{
    EXCEPTION_SCHEMA_VERSION, ExceptionPayload, StackFrame, StackTracePayload,
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
/// Uses `traceback.extract_stack()` to get the current stack frames,
/// excluding the logging infrastructure frames.
///
/// # Errors
///
/// Returns an error if Python traceback calls fail.
pub fn capture_stack(py: Python<'_>) -> PyResult<StackTracePayload> {
    let traceback = py.import("traceback")?;
    // extract_stack() returns a StackSummary (list of FrameSummary)
    // We skip the last few frames which are internal to the logging call
    let stack_summary = traceback.call_method0("extract_stack")?;
    let frames = extract_frames_from_stack_summary(py, &stack_summary)?;

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
fn build_exception_payload(
    py: Python<'_>,
    exc_value: &Bound<'_, PyAny>,
    traceback: Option<&Bound<'_, PyAny>>,
) -> PyResult<Option<ExceptionPayload>> {
    let traceback_mod = py.import("traceback")?;

    // Use TracebackException for comprehensive exception info
    let tb_exc_class = traceback_mod.getattr("TracebackException")?;

    // TracebackException.from_exception(exc, capture_locals=False)
    let kwargs = PyDict::new(py);
    kwargs.set_item("capture_locals", false)?;
    let tb_exc = tb_exc_class.call_method("from_exception", (exc_value,), Some(&kwargs))?;

    build_payload_from_traceback_exception(py, &tb_exc, traceback, Some(exc_value))
}

/// Build payload from a `TracebackException` object.
///
/// The `exc_value` parameter is the original exception instance. It's optional
/// because for chained exceptions we may not have direct access to the exception
/// instance (only the TracebackException).
fn build_payload_from_traceback_exception(
    py: Python<'_>,
    tb_exc: &Bound<'_, PyAny>,
    _traceback: Option<&Bound<'_, PyAny>>,
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
    let frames = extract_frames_from_tb_exception(py, tb_exc)?;

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
fn extract_notes_from_exc(exc: &Bound<'_, PyAny>) -> PyResult<Vec<String>> {
    let notes_attr = match exc.getattr("__notes__") {
        Ok(n) if !n.is_none() => n,
        _ => return Ok(Vec::new()),
    };

    let notes_list = notes_attr.downcast::<PyList>()?;
    let mut result = Vec::with_capacity(notes_list.len());
    for note in notes_list.iter() {
        result.push(note.str()?.extract()?);
    }
    Ok(result)
}

/// Extract stack frames from a TracebackException's stack attribute.
fn extract_frames_from_tb_exception(
    py: Python<'_>,
    tb_exc: &Bound<'_, PyAny>,
) -> PyResult<Vec<StackFrame>> {
    let stack = tb_exc.getattr("stack")?;
    extract_frames_from_stack_summary(py, &stack)
}

/// Extract stack frames from a StackSummary (list of FrameSummary).
fn extract_frames_from_stack_summary(
    _py: Python<'_>,
    stack_summary: &Bound<'_, PyAny>,
) -> PyResult<Vec<StackFrame>> {
    let list = stack_summary.downcast::<PyList>()?;
    let mut frames = Vec::with_capacity(list.len());

    for frame_summary in list.iter() {
        frames.push(frame_summary_to_stack_frame(&frame_summary)?);
    }

    Ok(frames)
}

/// Convert a Python FrameSummary to a Rust StackFrame.
fn frame_summary_to_stack_frame(frame: &Bound<'_, PyAny>) -> PyResult<StackFrame> {
    let filename: String = frame.getattr("filename")?.extract()?;
    let lineno: u32 = frame.getattr("lineno")?.extract()?;
    let function: String = frame.getattr("name")?.extract()?;

    // Python 3.11+ enhanced traceback info
    let end_lineno: Option<u32> = frame
        .getattr("end_lineno")
        .ok()
        .and_then(|v| if v.is_none() { None } else { v.extract().ok() });

    let colno: Option<u32> = frame
        .getattr("colno")
        .ok()
        .and_then(|v| if v.is_none() { None } else { v.extract().ok() });

    let end_colno: Option<u32> = frame
        .getattr("end_colno")
        .ok()
        .and_then(|v| if v.is_none() { None } else { v.extract().ok() });

    let source_line: Option<String> = frame
        .getattr("line")
        .ok()
        .and_then(|v| if v.is_none() { None } else { v.extract().ok() });

    // Locals are only present if capture_locals=True was used
    let locals: Option<BTreeMap<String, String>> = frame
        .getattr("locals")
        .ok()
        .and_then(|v| {
            if v.is_none() {
                return None;
            }
            let dict = v.downcast::<PyDict>().ok()?;
            let mut map = BTreeMap::new();
            for (key, value) in dict.iter() {
                let k: String = key.extract().ok()?;
                let v: String = value.repr().ok()?.extract().ok()?;
                map.insert(k, v);
            }
            Some(map)
        })
        .filter(|m| !m.is_empty());

    Ok(StackFrame {
        filename,
        lineno,
        end_lineno,
        colno,
        end_colno,
        function,
        source_line,
        locals,
    })
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
    build_payload_from_traceback_exception(py, &chained, None, None)?
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
        if let Some(payload) =
            build_payload_from_traceback_exception(py, &nested_tb_exc, None, None)?
        {
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    fn capture_exception_with_true_no_active_exception() {
        Python::with_gil(|py| {
            let true_val = PyBool::new(py, true);
            let result = capture_exception(py, true_val.as_any()).unwrap();
            assert!(result.is_none(), "No active exception should return None");
        });
    }

    #[rstest]
    fn capture_exception_with_false_returns_none() {
        Python::with_gil(|py| {
            let false_val = PyBool::new(py, false);
            let result = capture_exception(py, false_val.as_any()).unwrap();
            assert!(result.is_none());
        });
    }

    #[rstest]
    fn capture_exception_with_instance() {
        Python::with_gil(|py| {
            // Create an exception instance
            let exc = py
                .import("builtins")
                .unwrap()
                .getattr("ValueError")
                .unwrap()
                .call1(("test error",))
                .unwrap();

            let result = capture_exception(py, &exc).unwrap();
            assert!(result.is_some());

            let payload = result.unwrap();
            assert_eq!(payload.type_name, "ValueError");
            assert_eq!(payload.message, "test error");
            assert_eq!(payload.schema_version, EXCEPTION_SCHEMA_VERSION);
        });
    }

    #[rstest]
    fn capture_exception_with_tuple() {
        Python::with_gil(|py| {
            // Create a 3-tuple (type, value, traceback)
            let exc_type = py.import("builtins").unwrap().getattr("KeyError").unwrap();
            let exc_value = exc_type.call1(("missing_key",)).unwrap();
            let exc_tb = py.None();

            let tuple = PyTuple::new(
                py,
                &[exc_type.as_any(), exc_value.as_any(), exc_tb.bind(py)],
            )
            .unwrap();

            let result = capture_exception(py, tuple.as_any()).unwrap();
            assert!(result.is_some());

            let payload = result.unwrap();
            assert_eq!(payload.type_name, "KeyError");
        });
    }

    #[rstest]
    fn capture_exception_with_none_value_tuple() {
        Python::with_gil(|py| {
            // 3-tuple with None value means no exception
            let none = py.None();
            let tuple = PyTuple::new(py, &[none.bind(py), none.bind(py), none.bind(py)]).unwrap();

            let result = capture_exception(py, tuple.as_any()).unwrap();
            assert!(result.is_none());
        });
    }

    #[rstest]
    fn capture_exception_invalid_type_raises_error() {
        Python::with_gil(|py| {
            let code = c"42";
            let invalid = py.eval(code, None, None).unwrap();
            let result = capture_exception(py, &invalid);
            assert!(result.is_err());
        });
    }

    #[rstest]
    fn capture_exception_with_chained_cause() {
        Python::with_gil(|py| {
            let code =
                c"try:\n    raise IOError('read failed')\nexcept IOError as e:\n    raise RuntimeError('operation failed') from e\n";

            // Execute code and capture the exception
            let result = py.run(code, None, None);
            assert!(result.is_err());

            let err = result.unwrap_err();
            let exc_value = err.value(py);

            let payload = capture_exception(py, exc_value).unwrap().unwrap();

            assert_eq!(payload.type_name, "RuntimeError");
            assert!(payload.cause.is_some());

            let cause = payload.cause.unwrap();
            // IOError is an alias for OSError in Python 3
            assert_eq!(cause.type_name, "OSError");
            assert_eq!(cause.message, "read failed");
        });
    }

    #[rstest]
    fn capture_stack_returns_frames() {
        Python::with_gil(|py| {
            let payload = capture_stack(py).unwrap();
            assert_eq!(payload.schema_version, EXCEPTION_SCHEMA_VERSION);
            assert!(!payload.frames.is_empty(), "Stack should have frames");

            // Check that frames have required fields
            let frame = &payload.frames[0];
            assert!(!frame.filename.is_empty());
            assert!(!frame.function.is_empty());
        });
    }

    #[rstest]
    fn capture_exception_with_notes() {
        Python::with_gil(|py| {
            // Create an exception with notes (Python 3.11+)
            let code = c"e = ValueError('test'); e.add_note('Note 1'); e.add_note('Note 2')";
            let globals = PyDict::new(py);
            py.run(code, Some(&globals), None).unwrap();
            let exc = globals.get_item("e").unwrap().unwrap();

            let payload = capture_exception(py, &exc).unwrap().unwrap();

            assert_eq!(payload.notes.len(), 2);
            assert_eq!(payload.notes[0], "Note 1");
            assert_eq!(payload.notes[1], "Note 2");
        });
    }

    #[rstest]
    fn capture_exception_args_repr() {
        Python::with_gil(|py| {
            let exc = py
                .import("builtins")
                .unwrap()
                .getattr("ValueError")
                .unwrap()
                .call1(("message", 42))
                .unwrap();

            let payload = capture_exception(py, &exc).unwrap().unwrap();

            assert_eq!(payload.args_repr.len(), 2);
            assert_eq!(payload.args_repr[0], "'message'");
            assert_eq!(payload.args_repr[1], "42");
        });
    }

    #[rstest]
    fn types_are_send_and_sync() {
        // Verify that capture functions can be used across threads
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        assert_send::<ExceptionPayload>();
        assert_sync::<ExceptionPayload>();
        assert_send::<StackTracePayload>();
        assert_sync::<StackTracePayload>();
    }
}
