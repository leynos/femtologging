//! Stack frame extraction utilities for Python tracebacks.
//!
//! This module provides functions to extract stack frame information from
//! Python `FrameSummary` objects and convert them to the Rust `StackFrame`
//! type defined in [`crate::exception_schema`].

use log::trace;
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

/// Extract stack frames from a `TracebackException`'s `stack` attribute.
///
/// Retrieves the `stack` attribute from the provided `TracebackException` and
/// delegates to [`extract_frames_from_stack_summary`] for conversion.
///
/// # Errors
///
/// Returns an error in the following cases:
/// - `PyAttributeError` if `tb_exc` lacks a `stack` attribute
/// - `PyDowncastError` or `PyTypeError` if the `stack` cannot be converted to a
///   list of `FrameSummary` objects (propagated from `extract_frames_from_stack_summary`)
/// - Any extraction error from individual frame conversion
pub(crate) fn extract_frames_from_tb_exception(
    tb_exc: &Bound<'_, PyAny>,
) -> PyResult<Vec<StackFrame>> {
    let stack = tb_exc.getattr("stack")?;
    extract_frames_from_stack_summary(&stack)
}

/// Extract stack frames from a `StackSummary` (list of `FrameSummary`).
///
/// # Errors
///
/// Returns an error if `stack_summary` cannot be downcast to a list or if any
/// individual frame fails to convert (e.g., missing required attributes).
pub(crate) fn extract_frames_from_stack_summary(
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

/// Tracks counts of entries skipped during locals extraction.
#[derive(Default)]
struct LocalsSkipCounts {
    /// Entries skipped because the key was not a string.
    non_string_keys: usize,
    /// Entries skipped because repr() failed or could not be extracted.
    repr_failures: usize,
}

impl LocalsSkipCounts {
    /// Returns true if any entries were skipped.
    fn any_skipped(&self) -> bool {
        self.non_string_keys > 0 || self.repr_failures > 0
    }

    /// Returns the total number of skipped entries.
    fn total(&self) -> usize {
        self.non_string_keys + self.repr_failures
    }
}

/// Extract the locals dictionary from a frame, converting values to repr strings.
///
/// Skips individual entries that fail to extract rather than discarding the
/// entire dictionary, ensuring partial data is preserved when possible.
///
/// When trace-level logging is enabled, emits a summary of any skipped entries
/// at the end of extraction, categorising them by reason (non-string key or
/// repr failure).
pub(crate) fn extract_locals_dict(frame: &Bound<'_, PyAny>) -> Option<BTreeMap<String, String>> {
    let locals_attr = frame.getattr("locals").ok()?;
    if locals_attr.is_none() {
        return None;
    }
    let dict = locals_attr.downcast::<PyDict>().ok()?;
    let total_entries = dict.len();
    let mut map = BTreeMap::new();
    let mut skip_counts = LocalsSkipCounts::default();

    for (key, value) in dict.iter() {
        let Some(k) = key.extract::<String>().ok() else {
            skip_counts.non_string_keys += 1;
            continue;
        };
        let Some(v) = value.repr().ok().and_then(|r| r.extract::<String>().ok()) else {
            skip_counts.repr_failures += 1;
            continue;
        };
        map.insert(k, v);
    }

    if skip_counts.any_skipped() {
        trace!(
            concat!(
                "extract_locals_dict: skipped {}/{} entries ",
                "(non-string keys: {}, repr failures: {})"
            ),
            skip_counts.total(),
            total_entries,
            skip_counts.non_string_keys,
            skip_counts.repr_failures
        );
    }

    if map.is_empty() { None } else { Some(map) }
}
