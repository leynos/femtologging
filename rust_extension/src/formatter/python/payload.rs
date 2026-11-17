//! Convert exception and stack payloads into their Python mapping schema.
//!
//! [`record_to_dict`](super::record_to_dict) attaches these conversions to the
//! record dict that Python handlers and formatters receive. Every list and
//! nested payload is rebuilt here rather than shared, so the Python view is a
//! plain, pickle-safe structure.
//!
//! A truncation marker is converted field by field. Defaults are omitted, which
//! keeps the conversion bounded: the dict has at most one entry per marker
//! field however much data capture discarded.

use pyo3::{
    prelude::*,
    types::{PyDict, PyList},
};

use crate::exception_schema::{ExceptionPayload, PayloadTruncation, StackFrame, StackTracePayload};

/// Convert a `StackTracePayload` to a Python dict.
pub(super) fn stack_payload_to_py(
    py: Python<'_>,
    payload: &StackTracePayload,
) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item("schema_version", payload.schema_version)?;
    dict.set_item("frames", frames_to_pylist(py, &payload.frames)?)?;
    set_truncation_item(py, &dict, payload.truncation.as_ref())?;
    Ok(dict.into())
}

/// Convert an `ExceptionPayload` to a Python dict.
pub(super) fn exception_payload_to_py(
    py: Python<'_>,
    payload: &ExceptionPayload,
) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item("schema_version", payload.schema_version)?;
    dict.set_item("type_name", &payload.type_name)?;
    dict.set_item("message", &payload.message)?;

    set_optional_exception_items(py, &dict, payload)?;

    Ok(dict.into())
}

/// Set optional fields on the exception payload dict.
///
/// Each field is written only when the payload carries it, so the dict echoes
/// the exception exactly as Python raised it.
fn set_optional_exception_items(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    payload: &ExceptionPayload,
) -> PyResult<()> {
    set_optional_exception_item(dict, "module", payload.module.as_deref())?;
    set_string_list_item(py, dict, "args_repr", &payload.args_repr)?;
    set_string_list_item(py, dict, "notes", &payload.notes)?;
    set_frame_list_item(py, dict, &payload.frames)?;
    set_chained_exception_item(py, dict, "cause", payload.cause.as_deref())?;
    set_chained_exception_item(py, dict, "context", payload.context.as_deref())?;
    set_suppress_context_item(dict, payload.suppress_context)?;
    // Written last so the payload dict keeps the key order it has always had;
    // callers would otherwise observe `exceptions` before the chained fields.
    set_exception_member_item(py, dict, &payload.exceptions)?;
    set_truncation_item(py, dict, payload.truncation.as_ref())?;
    Ok(())
}

/// Set the `truncation` marker, unless the payload omitted nothing.
fn set_truncation_item(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    truncation: Option<&PayloadTruncation>,
) -> PyResult<()> {
    if let Some(marker) = truncation {
        dict.set_item("truncation", truncation_to_py(py, marker)?)?;
    }
    Ok(())
}

/// Set `key` when the exception carries a string value for it.
fn set_optional_exception_item(
    dict: &Bound<'_, PyDict>,
    key: &str,
    value: Option<&str>,
) -> PyResult<()> {
    if let Some(value) = value {
        dict.set_item(key, value)?;
    }
    Ok(())
}

/// Set `key` to a Python list of `values`, unless the list is empty.
fn set_string_list_item(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    key: &str,
    values: &[String],
) -> PyResult<()> {
    if !values.is_empty() {
        dict.set_item(key, string_vec_to_pylist(py, values)?)?;
    }
    Ok(())
}

/// Set the `frames` list, unless the exception carries no frames.
fn set_frame_list_item(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    frames: &[StackFrame],
) -> PyResult<()> {
    if !frames.is_empty() {
        dict.set_item("frames", frames_to_pylist(py, frames)?)?;
    }
    Ok(())
}

/// Set the `exceptions` list, unless the exception group has no members.
fn set_exception_member_item(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    exceptions: &[ExceptionPayload],
) -> PyResult<()> {
    if !exceptions.is_empty() {
        dict.set_item("exceptions", exceptions_to_pylist(py, exceptions)?)?;
    }
    Ok(())
}

/// Set the chained `cause` or `context` payload, when the exception has one.
fn set_chained_exception_item(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    key: &str,
    chained: Option<&ExceptionPayload>,
) -> PyResult<()> {
    if let Some(chained) = chained {
        dict.set_item(key, exception_payload_to_py(py, chained)?)?;
    }
    Ok(())
}

/// Record that the exception suppresses implicit context.
fn set_suppress_context_item(dict: &Bound<'_, PyDict>, suppress_context: bool) -> PyResult<()> {
    if suppress_context {
        dict.set_item("suppress_context", true)?;
    }
    Ok(())
}

/// Convert a `StackFrame` to a Python dict.
fn stack_frame_to_py(py: Python<'_>, frame: &StackFrame) -> PyResult<Py<PyAny>> {
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
    set_locals_item(py, &dict, frame)?;

    Ok(dict.into())
}

/// Attach the captured local variables, when capture kept any.
fn set_locals_item(py: Python<'_>, dict: &Bound<'_, PyDict>, frame: &StackFrame) -> PyResult<()> {
    let Some(ref locals) = frame.locals else {
        return Ok(());
    };
    let locals_dict = PyDict::new(py);
    for (key, value) in locals {
        locals_dict.set_item(key, value)?;
    }
    dict.set_item("locals", locals_dict)
}

/// Convert a truncation marker to a Python dict of its non-default fields.
///
/// Defaults are omitted so the Python view matches the serialized schema and
/// stays fixed-size: the dict has at most one entry per marker field.
fn truncation_to_py(py: Python<'_>, marker: &PayloadTruncation) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    set_flag(&dict, "message_truncated", marker.message_truncated)?;
    set_count(&dict, "args_omitted", marker.args_omitted)?;
    set_flag(&dict, "args_truncated", marker.args_truncated)?;
    set_count(&dict, "notes_omitted", marker.notes_omitted)?;
    set_flag(&dict, "notes_truncated", marker.notes_truncated)?;
    set_count(&dict, "frames_omitted", marker.frames_omitted)?;
    set_flag(&dict, "cause_omitted", marker.cause_omitted)?;
    set_flag(&dict, "context_omitted", marker.context_omitted)?;
    set_count(&dict, "exceptions_omitted", marker.exceptions_omitted)?;
    set_flag(&dict, "members_truncated", marker.members_truncated)?;
    set_count(&dict, "failed_elements", marker.failed_elements)?;
    set_flag(&dict, "budget_exhausted", marker.budget_exhausted)?;
    Ok(dict.into())
}

/// Set a boolean marker field when it is set.
fn set_flag(dict: &Bound<'_, PyDict>, key: &str, value: bool) -> PyResult<()> {
    if value {
        dict.set_item(key, true)?;
    }
    Ok(())
}

/// Set a counted marker field when it is positive.
fn set_count(dict: &Bound<'_, PyDict>, key: &str, value: u32) -> PyResult<()> {
    if value > 0 {
        dict.set_item(key, value)?;
    }
    Ok(())
}

/// Convert a `&[String]` to a Python list.
fn string_vec_to_pylist(py: Python<'_>, strings: &[String]) -> PyResult<Py<PyAny>> {
    let list = PyList::empty(py);
    for value in strings {
        list.append(value)?;
    }
    Ok(list.into())
}

/// Convert a `&[StackFrame]` to a Python list of dicts.
fn frames_to_pylist(py: Python<'_>, frames: &[StackFrame]) -> PyResult<Py<PyAny>> {
    let list = PyList::empty(py);
    for frame in frames {
        list.append(stack_frame_to_py(py, frame)?)?;
    }
    Ok(list.into())
}

/// Convert a `&[ExceptionPayload]` to a Python list of dicts.
fn exceptions_to_pylist(py: Python<'_>, exceptions: &[ExceptionPayload]) -> PyResult<Py<PyAny>> {
    let list = PyList::empty(py);
    for exception in exceptions {
        list.append(exception_payload_to_py(py, exception)?)?;
    }
    Ok(list.into())
}
