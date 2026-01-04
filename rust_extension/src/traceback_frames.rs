//! Stack frame extraction utilities for Python tracebacks.
//!
//! This module provides functions to extract stack frame information from
//! Python `FrameSummary` objects and convert them to the Rust `StackFrame`
//! type defined in [`crate::exception_schema`].

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};
use std::collections::BTreeMap;

use crate::exception_schema::StackFrame;

/// Get an optional attribute from a Python object, returning `None` if the
/// attribute doesn't exist or is Python `None`.
///
/// This function silently returns `None` in the following cases:
/// - The attribute does not exist on the object
/// - The attribute value is Python `None`
/// - The attribute exists but cannot be extracted to type `T`
pub(crate) fn get_optional_attr<'py, T>(obj: &Bound<'py, PyAny>, attr: &str) -> Option<T>
where
    T: FromPyObject<'py>,
{
    obj.getattr(attr)
        .ok()
        .filter(|v| !v.is_none())
        .and_then(|v| v.extract().ok())
}

/// Extract stack frames from a TracebackException's stack attribute.
pub(crate) fn extract_frames_from_tb_exception(
    py: Python<'_>,
    tb_exc: &Bound<'_, PyAny>,
) -> PyResult<Vec<StackFrame>> {
    let stack = tb_exc.getattr("stack")?;
    extract_frames_from_stack_summary(py, &stack)
}

/// Extract stack frames from a StackSummary (list of FrameSummary).
pub(crate) fn extract_frames_from_stack_summary(
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
    let end_lineno: Option<u32> = get_optional_attr(frame, "end_lineno");
    let colno: Option<u32> = get_optional_attr(frame, "colno");
    let end_colno: Option<u32> = get_optional_attr(frame, "end_colno");
    let source_line: Option<String> = get_optional_attr(frame, "line");

    // Locals are only present if capture_locals=True was used
    let locals: Option<BTreeMap<String, String>> = extract_locals_dict(frame);

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

/// Extract the locals dictionary from a frame, converting values to repr strings.
fn extract_locals_dict(frame: &Bound<'_, PyAny>) -> Option<BTreeMap<String, String>> {
    let locals_attr = frame.getattr("locals").ok()?;
    if locals_attr.is_none() {
        return None;
    }
    let dict = locals_attr.downcast::<PyDict>().ok()?;
    let mut map = BTreeMap::new();
    for (key, value) in dict.iter() {
        let k: String = key.extract().ok()?;
        let v: String = value.repr().ok()?.extract().ok()?;
        map.insert(k, v);
    }
    if map.is_empty() { None } else { Some(map) }
}
