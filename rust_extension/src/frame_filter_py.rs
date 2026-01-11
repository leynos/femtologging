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
    T: pyo3::FromPyObject<'py>,
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
        .downcast::<PyList>()
        .map_err(|_| PyTypeError::new_err("'frames' must be a list"))?;

    let mut frames = Vec::with_capacity(list.len());
    for item in list.iter() {
        let dict = item
            .downcast::<PyDict>()
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
) -> PyResult<PyObject> {
    let frames = extract_frames(payload)?;
    let had_frames_key = payload.contains("frames")?;
    let filtered = apply_filters(frames, opts);

    // Clone all keys from the original payload
    let result = payload.copy()?;

    // Update frames only if the original had them, or always set if filtering produced frames
    // Match the Rust serde behavior: skip_serializing_if = "Vec::is_empty"
    if had_frames_key {
        if filtered.is_empty() {
            // Remove empty frames to match serialisation semantics
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
fn filter_exception_payload(
    py: Python<'_>,
    payload: &Bound<'_, PyDict>,
    opts: &FilterOptions<'_>,
) -> PyResult<PyObject> {
    let frames = extract_frames(payload)?;
    let filtered = apply_filters(frames, opts);

    // Build result dict, copying all keys
    let result = PyDict::new(py);

    // Copy simple fields
    for key in [
        "schema_version",
        "type_name",
        "module",
        "message",
        "suppress_context",
    ] {
        if let Some(value) = payload.get_item(key)? {
            result.set_item(key, value)?;
        }
    }

    // Copy list fields
    for key in ["args_repr", "notes"] {
        if let Some(value) = payload.get_item(key)? {
            result.set_item(key, value)?;
        }
    }

    // Set filtered frames
    if !filtered.is_empty() {
        result.set_item("frames", frames_to_py_list(py, &filtered)?)?;
    }

    // Recursively filter cause
    if let Some(cause) = payload.get_item("cause")? {
        let cause_dict = cause.downcast::<PyDict>()?;
        let filtered_cause = filter_exception_payload(py, cause_dict, opts)?;
        result.set_item("cause", filtered_cause)?;
    }

    // Recursively filter context
    if let Some(context) = payload.get_item("context")? {
        let context_dict = context.downcast::<PyDict>()?;
        let filtered_context = filter_exception_payload(py, context_dict, opts)?;
        result.set_item("context", filtered_context)?;
    }

    // Recursively filter exception group members
    if let Some(exceptions) = payload.get_item("exceptions")? {
        let exceptions_list = exceptions.downcast::<PyList>()?;
        let filtered_list = PyList::empty(py);
        for exc in exceptions_list.iter() {
            let exc_dict = exc.downcast::<PyDict>()?;
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
fn is_exception_payload(payload: &Bound<'_, PyDict>) -> bool {
    payload.contains("type_name").unwrap_or(false) && payload.contains("message").unwrap_or(false)
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
) -> PyResult<PyObject> {
    let opts = FilterOptions {
        exclude_filenames: exclude_filenames.as_deref(),
        exclude_functions: exclude_functions.as_deref(),
        max_depth,
        exclude_logging,
    };

    if is_exception_payload(payload) {
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
mod tests {
    //! Tests for Python frame filter bindings.

    use super::*;
    use rstest::rstest;
    use serial_test::serial;

    fn make_stack_payload_dict<'py>(py: Python<'py>, filenames: &[&str]) -> Bound<'py, PyDict> {
        let frames = PyList::empty(py);
        for (i, filename) in filenames.iter().enumerate() {
            let frame = PyDict::new(py);
            frame
                .set_item("filename", *filename)
                .expect("failed to set frame filename");
            frame
                .set_item("lineno", i as u32 + 1)
                .expect("failed to set frame lineno");
            frame
                .set_item("function", format!("func_{i}"))
                .expect("failed to set frame function");
            frames.append(frame).expect("failed to append frame");
        }
        let payload = PyDict::new(py);
        payload
            .set_item("schema_version", 1u16)
            .expect("failed to set schema_version");
        payload
            .set_item("frames", frames)
            .expect("failed to set frames");
        payload
    }

    fn make_exception_payload_dict<'py>(py: Python<'py>, filenames: &[&str]) -> Bound<'py, PyDict> {
        let payload = make_stack_payload_dict(py, filenames);
        payload
            .set_item("type_name", "ValueError")
            .expect("failed to set type_name");
        payload
            .set_item("message", "test error")
            .expect("failed to set message");
        payload
    }

    /// Helper to call filter_frames and extract the resulting frames list.
    fn filter_and_extract_frames<'py>(
        py: Python<'py>,
        payload: &Bound<'py, PyDict>,
        exclude_filenames: Option<Vec<String>>,
        exclude_functions: Option<Vec<String>>,
        max_depth: Option<usize>,
        exclude_logging: bool,
    ) -> Bound<'py, PyList> {
        let result = filter_frames(
            py,
            payload,
            exclude_filenames,
            exclude_functions,
            max_depth,
            exclude_logging,
        )
        .expect("filter_frames failed");
        let result_dict = result
            .downcast_bound::<PyDict>(py)
            .expect("result is not a dict");
        let frames = result_dict
            .get_item("frames")
            .expect("failed to get frames key")
            .expect("frames key is None");
        frames
            .downcast::<PyList>()
            .expect("frames is not a list")
            .clone()
    }

    #[rstest]
    #[serial]
    fn filter_stack_payload_exclude_logging() {
        Python::with_gil(|py| {
            let payload = make_stack_payload_dict(
                py,
                &[
                    "myapp/main.py",
                    "femtologging/__init__.py",
                    "logging/__init__.py",
                ],
            );

            let frames_list = filter_and_extract_frames(py, &payload, None, None, None, true);

            assert_eq!(frames_list.len(), 1);
            let frame = frames_list.get_item(0).expect("failed to get first frame");
            let frame_dict = frame.downcast::<PyDict>().expect("frame is not a dict");
            let filename: String = frame_dict
                .get_item("filename")
                .expect("failed to get filename key")
                .expect("filename key is None")
                .extract()
                .expect("failed to extract filename");
            assert_eq!(filename, "myapp/main.py");
        });
    }

    #[rstest]
    #[serial]
    fn filter_stack_payload_exclude_filenames() {
        Python::with_gil(|py| {
            let payload = make_stack_payload_dict(
                py,
                &["myapp/main.py", ".venv/lib/requests.py", "myapp/utils.py"],
            );

            let frames_list = filter_and_extract_frames(
                py,
                &payload,
                Some(vec![".venv/".to_string()]),
                None,
                None,
                false,
            );

            assert_eq!(frames_list.len(), 2);
        });
    }

    #[rstest]
    #[serial]
    fn filter_stack_payload_max_depth() {
        Python::with_gil(|py| {
            let payload = make_stack_payload_dict(py, &["a.py", "b.py", "c.py", "d.py", "e.py"]);

            let frames_list = filter_and_extract_frames(py, &payload, None, None, Some(2), false);

            assert_eq!(frames_list.len(), 2);
            // Should be the last 2 frames (d.py, e.py)
            let frame0 = frames_list.get_item(0).expect("failed to get first frame");
            let frame0_dict = frame0.downcast::<PyDict>().expect("frame is not a dict");
            let filename0: String = frame0_dict
                .get_item("filename")
                .expect("failed to get filename key")
                .expect("filename key is None")
                .extract()
                .expect("failed to extract filename");
            assert_eq!(filename0, "d.py");
        });
    }

    #[rstest]
    #[serial]
    fn filter_exception_payload_detects_type() {
        Python::with_gil(|py| {
            let payload =
                make_exception_payload_dict(py, &["myapp/main.py", "femtologging/__init__.py"]);

            assert!(is_exception_payload(&payload));

            let result =
                filter_frames(py, &payload, None, None, None, true).expect("filter_frames failed");
            let result_dict = result
                .downcast_bound::<PyDict>(py)
                .expect("result is not a dict");

            // Should preserve exception fields
            let type_name: String = result_dict
                .get_item("type_name")
                .expect("failed to get type_name key")
                .expect("type_name key is None")
                .extract()
                .expect("failed to extract type_name");
            assert_eq!(type_name, "ValueError");

            let frames = result_dict
                .get_item("frames")
                .expect("failed to get frames key")
                .expect("frames key is None");
            let frames_list = frames.downcast::<PyList>().expect("frames is not a list");
            assert_eq!(frames_list.len(), 1);
        });
    }

    #[rstest]
    #[serial]
    fn filter_exception_payload_with_cause() {
        Python::with_gil(|py| {
            let cause = make_exception_payload_dict(py, &["cause.py", "femtologging/__init__.py"]);
            cause
                .set_item("type_name", "IOError")
                .expect("failed to set type_name");
            cause
                .set_item("message", "cause error")
                .expect("failed to set message");

            let payload = make_exception_payload_dict(py, &["main.py", "logging/__init__.py"]);
            payload
                .set_item("cause", cause)
                .expect("failed to set cause");

            let result =
                filter_frames(py, &payload, None, None, None, true).expect("filter_frames failed");
            let result_dict = result
                .downcast_bound::<PyDict>(py)
                .expect("result is not a dict");

            // Check main frames filtered
            let frames = result_dict
                .get_item("frames")
                .expect("failed to get frames key")
                .expect("frames key is None");
            let frames_list = frames.downcast::<PyList>().expect("frames is not a list");
            assert_eq!(frames_list.len(), 1);

            // Check cause frames also filtered
            let cause_result = result_dict
                .get_item("cause")
                .expect("failed to get cause key")
                .expect("cause key is None");
            let cause_dict = cause_result
                .downcast::<PyDict>()
                .expect("cause is not a dict");
            let cause_frames = cause_dict
                .get_item("frames")
                .expect("failed to get cause frames key")
                .expect("cause frames key is None");
            let cause_frames_list = cause_frames
                .downcast::<PyList>()
                .expect("cause frames is not a list");
            assert_eq!(cause_frames_list.len(), 1);
        });
    }

    #[rstest]
    #[serial]
    fn get_logging_patterns_returns_expected() {
        let patterns = get_logging_infrastructure_patterns();
        assert!(patterns.contains(&"femtologging"));
        assert!(patterns.contains(&"logging/__init__"));
    }
}
