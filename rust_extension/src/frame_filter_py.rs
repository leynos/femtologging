//! Python bindings for frame filtering utilities.
//!
//! This module provides Python-callable functions for filtering stack frames
//! from exception and stack trace payloads. The functions operate on Python
//! dicts (the same format returned by `handle_record`).

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use crate::exception_schema::StackFrame;
use crate::frame_filter::{
    LOGGING_INFRA_PATTERNS, exclude_by_filename, exclude_by_function,
    exclude_logging_infrastructure, limit_frames,
};

/// Filter options encapsulating all filtering parameters.
///
/// This struct groups related filter parameters to reduce function argument counts
/// and improve code clarity.
struct FilterOptions<'a> {
    exclude_filenames: Option<&'a [String]>,
    exclude_functions: Option<&'a [String]>,
    max_depth: Option<usize>,
    exclude_logging: bool,
}

/// Helper to extract an optional field with type error reporting.
///
/// If the field is present but has the wrong type, returns a TypeError instead
/// of silently dropping the value.
fn extract_optional<'py, T>(dict: &Bound<'py, PyDict>, key: &str) -> PyResult<Option<T>>
where
    T: for<'a> pyo3::FromPyObject<'a, 'py>,
{
    match dict.get_item(key)? {
        Some(v) if v.is_none() => Ok(None),
        Some(v) => v
            .extract()
            .map(Some)
            .map_err(|_| PyTypeError::new_err(format!("frame dict key '{}' has wrong type", key))),
        None => Ok(None),
    }
}

/// Extract a StackFrame from a Python dict.
fn dict_to_stack_frame(dict: &Bound<'_, PyDict>) -> PyResult<StackFrame> {
    let filename: String = dict
        .get_item("filename")?
        .ok_or_else(|| PyTypeError::new_err("frame dict missing 'filename' key"))?
        .extract()?;

    let lineno: u32 = dict
        .get_item("lineno")?
        .ok_or_else(|| PyTypeError::new_err("frame dict missing 'lineno' key"))?
        .extract()?;

    let function: String = dict
        .get_item("function")?
        .ok_or_else(|| PyTypeError::new_err("frame dict missing 'function' key"))?
        .extract()?;

    let end_lineno: Option<u32> = extract_optional(dict, "end_lineno")?;
    let colno: Option<u32> = extract_optional(dict, "colno")?;
    let end_colno: Option<u32> = extract_optional(dict, "end_colno")?;
    let source_line: Option<String> = extract_optional(dict, "source_line")?;

    // Extract locals if present
    let locals: Option<std::collections::BTreeMap<String, String>> =
        extract_optional(dict, "locals")?;

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

/// Convert a StackFrame back to a Python dict.
fn stack_frame_to_dict<'py>(py: Python<'py>, frame: &StackFrame) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("filename", &frame.filename)?;
    dict.set_item("lineno", frame.lineno)?;
    dict.set_item("function", &frame.function)?;

    if let Some(end_lineno) = frame.end_lineno {
        dict.set_item("end_lineno", end_lineno)?;
    }
    if let Some(colno) = frame.colno {
        dict.set_item("colno", colno)?;
    }
    if let Some(end_colno) = frame.end_colno {
        dict.set_item("end_colno", end_colno)?;
    }
    if let Some(ref source_line) = frame.source_line {
        dict.set_item("source_line", source_line)?;
    }
    if let Some(ref locals) = frame.locals {
        dict.set_item("locals", locals.clone())?;
    }

    Ok(dict)
}

/// Extract frames from a payload dict's 'frames' key.
fn extract_frames(payload: &Bound<'_, PyDict>) -> PyResult<Vec<StackFrame>> {
    let frames_list = match payload.get_item("frames")? {
        Some(list) => list,
        None => return Ok(Vec::new()),
    };

    let list = frames_list
        .cast::<PyList>()
        .map_err(|_| PyTypeError::new_err("'frames' must be a list"))?;

    let mut frames = Vec::with_capacity(list.len());
    for item in list.iter() {
        let dict = item
            .cast::<PyDict>()
            .map_err(|_| PyTypeError::new_err("each frame must be a dict"))?;
        frames.push(dict_to_stack_frame(dict)?);
    }

    Ok(frames)
}

/// Convert a list of StackFrames back to a Python list of dicts.
fn frames_to_py_list<'py>(py: Python<'py>, frames: &[StackFrame]) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::empty(py);
    for frame in frames {
        list.append(stack_frame_to_dict(py, frame)?)?;
    }
    Ok(list)
}

/// Apply filtering options to a list of frames.
fn apply_filters(frames: Vec<StackFrame>, opts: &FilterOptions<'_>) -> Vec<StackFrame> {
    let mut result = frames;

    // Apply filename exclusions
    if let Some(patterns) = opts.exclude_filenames {
        let pattern_refs: Vec<&str> = patterns.iter().map(|s| s.as_str()).collect();
        result = exclude_by_filename(&result, &pattern_refs);
    }

    // Apply function exclusions
    if let Some(patterns) = opts.exclude_functions {
        let pattern_refs: Vec<&str> = patterns.iter().map(|s| s.as_str()).collect();
        result = exclude_by_function(&result, &pattern_refs);
    }

    // Apply logging infrastructure filter
    if opts.exclude_logging {
        result = exclude_logging_infrastructure(&result);
    }

    // Apply depth limit
    if let Some(n) = opts.max_depth {
        result = limit_frames(&result, n);
    }

    result
}

/// Filter a stack_info payload dict.
///
/// Preserves all keys from the original payload, only updating the frames.
fn filter_stack_payload(
    py: Python<'_>,
    payload: &Bound<'_, PyDict>,
    opts: &FilterOptions<'_>,
) -> PyResult<Py<PyAny>> {
    let frames = extract_frames(payload)?;
    let had_frames_key = payload.contains("frames")?;
    let filtered = apply_filters(frames, opts);

    // Clone all keys from the original payload
    let result = payload.copy()?;

    // Update frames only if the original had them, or always set if filtering produced frames
    // Match the Rust serde behavior: skip_serializing_if = "Vec::is_empty"
    if had_frames_key {
        if filtered.is_empty() {
            // Remove empty frames to match serialization semantics
            result.del_item("frames").ok();
        } else {
            result.set_item("frames", frames_to_py_list(py, &filtered)?)?;
        }
    } else if !filtered.is_empty() {
        result.set_item("frames", frames_to_py_list(py, &filtered)?)?;
    }

    Ok(result.into())
}

/// Filter an exc_info payload dict, recursively filtering cause/context/exceptions.
///
/// Preserves all keys from the original payload, only updating the frames
/// and recursively filtering cause/context/exceptions.
fn filter_exception_payload(
    py: Python<'_>,
    payload: &Bound<'_, PyDict>,
    opts: &FilterOptions<'_>,
) -> PyResult<Py<PyAny>> {
    let frames = extract_frames(payload)?;
    let had_frames_key = payload.contains("frames")?;
    let filtered = apply_filters(frames, opts);

    // Clone all keys from the original payload (preserves unknown keys)
    let result = payload.copy()?;

    // Update frames only if the original had them
    // Match the Rust serde behavior: skip_serializing_if = "Vec::is_empty"
    if had_frames_key {
        if filtered.is_empty() {
            // Remove empty frames to match serialization semantics
            result.del_item("frames").ok();
        } else {
            result.set_item("frames", frames_to_py_list(py, &filtered)?)?;
        }
    } else if !filtered.is_empty() {
        result.set_item("frames", frames_to_py_list(py, &filtered)?)?;
    }

    // Recursively filter cause
    if let Some(cause) = payload.get_item("cause")? {
        let cause_dict = cause
            .cast::<PyDict>()
            .map_err(|_| PyTypeError::new_err("'cause' must be a dict"))?;
        let filtered_cause = filter_exception_payload(py, cause_dict, opts)?;
        result.set_item("cause", filtered_cause)?;
    }

    // Recursively filter context
    if let Some(context) = payload.get_item("context")? {
        let context_dict = context
            .cast::<PyDict>()
            .map_err(|_| PyTypeError::new_err("'context' must be a dict"))?;
        let filtered_context = filter_exception_payload(py, context_dict, opts)?;
        result.set_item("context", filtered_context)?;
    }

    // Recursively filter exception group members
    if let Some(exceptions) = payload.get_item("exceptions")? {
        let exceptions_list = exceptions
            .cast::<PyList>()
            .map_err(|_| PyTypeError::new_err("'exceptions' must be a list"))?;
        let filtered_list = PyList::empty(py);
        for exc in exceptions_list.iter() {
            let exc_dict = exc
                .cast::<PyDict>()
                .map_err(|_| PyTypeError::new_err("each exception must be a dict"))?;
            let filtered_exc = filter_exception_payload(py, exc_dict, opts)?;
            filtered_list.append(filtered_exc)?;
        }
        result.set_item("exceptions", filtered_list)?;
    }

    Ok(result.into())
}

/// Detect whether a payload is an exception payload or stack payload.
///
/// Exception payloads have 'type_name' and 'message' keys.
///
/// # Errors
///
/// Returns an error if the dict membership check fails.
fn is_exception_payload(payload: &Bound<'_, PyDict>) -> PyResult<bool> {
    Ok(payload.contains("type_name")? && payload.contains("message")?)
}

/// Filter frames from a stack_info or exc_info payload.
///
/// Parameters
/// ----------
/// payload : dict
///     The stack_info or exc_info dict from a log record.
/// exclude_filenames : list[str], optional
///     Filename patterns to exclude (substring matching).
/// exclude_functions : list[str], optional
///     Function name patterns to exclude (substring matching).
/// max_depth : int, optional
///     Maximum number of frames to retain (keeps most recent).
/// exclude_logging : bool, default False
///     If True, exclude common logging infrastructure frames
///     (femtologging, logging module internals).
///
/// Returns
/// -------
/// dict
///     A new payload dict with frames filtered.
///
/// Examples
/// --------
/// ```python
/// # In a custom handler's handle_record method:
/// def handle_record(self, record: dict) -> None:
///     if exc := record.get("exc_info"):
///         filtered = filter_frames(exc, exclude_logging=True, max_depth=10)
///         # Use filtered payload...
/// ```
#[pyfunction]
#[pyo3(signature = (payload, *, exclude_filenames=None, exclude_functions=None, max_depth=None, exclude_logging=false))]
pub fn filter_frames(
    py: Python<'_>,
    payload: &Bound<'_, PyDict>,
    exclude_filenames: Option<Vec<String>>,
    exclude_functions: Option<Vec<String>>,
    max_depth: Option<usize>,
    exclude_logging: bool,
) -> PyResult<Py<PyAny>> {
    let opts = FilterOptions {
        exclude_filenames: exclude_filenames.as_deref(),
        exclude_functions: exclude_functions.as_deref(),
        max_depth,
        exclude_logging,
    };

    if is_exception_payload(payload)? {
        filter_exception_payload(py, payload, &opts)
    } else {
        filter_stack_payload(py, payload, &opts)
    }
}

/// Return the list of filename patterns used by exclude_logging.
///
/// This is useful for inspecting or extending the default patterns.
///
/// Returns
/// -------
/// list[str]
///     The default logging infrastructure filename patterns.
#[pyfunction]
pub fn get_logging_infrastructure_patterns() -> Vec<&'static str> {
    LOGGING_INFRA_PATTERNS.to_vec()
}

#[cfg(test)]
#[path = "frame_filter_py_tests.rs"]
mod tests;
