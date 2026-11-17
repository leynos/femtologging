//! Helpers for adapting Python callables into [`FemtoFormatter`] instances.
//!
//! `formatter_from_py` validates a Python object with
//! `PythonFormatter::try_new` and returns it as a `SharedFormatter`, so the
//! logger can call a Python function or a `format(record)` method without
//! knowing it is Python. `record_to_dict` is the conversion both that adapter
//! and Python handlers share: it renders a `FemtoLogRecord` and its
//! `RecordMetadata` as a dict and attaches the exception and stack payloads
//! through `exception_payload_to_py` and `stack_payload_to_py`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use pyo3::{
    exceptions::PyTypeError,
    prelude::*,
    types::{PyDict, PyList, PyString},
};

use crate::exception_schema::{ExceptionPayload, StackFrame, StackTracePayload};
use crate::log_record::{FemtoLogRecord, RecordMetadata};
use crate::python::fq_py_type;

use super::{FemtoFormatter, SharedFormatter};

/// Lets Rust invoke a Python callable as a [`FemtoFormatter`] implementation.
#[derive(Clone)]
struct PythonFormatter {
    /// Owned Python callable protected by a mutex while a worker clones it under
    /// the GIL for invocation.
    callable: Arc<Mutex<Py<PyAny>>>,
    /// Cached fully qualified Python type name used in formatter error messages.
    description: String,
}

impl PythonFormatter {
    /// Validate `obj` and retain either it or its callable `format` method.
    fn try_new(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        let description = fq_py_type(obj);
        if let Ok(s) = obj.cast::<PyString>() {
            let msg = format!(
                "formatter must be callable or provide a callable format() method (got string: {s})",
            );
            return Err(PyTypeError::new_err(msg));
        }
        let callable = if obj.is_callable() {
            obj.clone().unbind()
        } else {
            let format = obj.getattr("format").map_err(|_| {
                PyTypeError::new_err(format!(
                    "formatter must be callable or provide a callable format() method (got Python type: {description})",
                ))
            })?;
            if !format.is_callable() {
                return Err(PyTypeError::new_err(format!(
                    "formatter.format must be callable (got Python type: {description})",
                )));
            }
            format.clone().unbind()
        };
        Ok(Self {
            callable: Arc::new(Mutex::new(callable)),
            description,
        })
    }

    /// Reacquire the GIL, convert the record, invoke the owned callable, and
    /// extract its string result; Python exceptions are returned unchanged.
    fn call(&self, record: &FemtoLogRecord) -> PyResult<String> {
        Python::attach(|py| {
            let payload = record_to_dict(py, record)?;
            let callable = {
                let guard = self.callable.lock().map_err(|_| {
                    pyo3::exceptions::PyRuntimeError::new_err(
                        "Python formatter mutex poisoned by prior panic",
                    )
                })?;
                guard.clone_ref(py)
            };
            let result = callable.call1(py, (payload,))?;
            result.extract::<String>(py)
        })
    }
}

/// Convert a [`FemtoLogRecord`] to a Python dict for use by Python handlers/formatters.
///
/// This function is used by both the Python formatter adapter and the
/// `handle_record` hook for Python handlers.
pub fn record_to_dict(py: Python<'_>, record: &FemtoLogRecord) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item("logger", record.logger())?;
    dict.set_item("level", record.level_str())?;
    dict.set_item("message", record.message())?;
    dict.set_item("levelno", u8::from(record.level()))?;
    dict.set_item("metadata", record_metadata_to_dict(py, record.metadata())?)?;
    attach_payloads(py, &dict, record)?;
    Ok(dict.into())
}

/// Convert a record's provenance metadata into a Python dict.
fn record_metadata_to_dict<'py>(
    py: Python<'py>,
    metadata: &RecordMetadata,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("module_path", &metadata.module_path)?;
    dict.set_item("filename", &metadata.filename)?;
    dict.set_item("line_number", metadata.line_number)?;
    dict.set_item("thread_name", &metadata.thread_name)?;
    dict.set_item("thread_id", format!("{:?}", metadata.thread_id))?;
    dict.set_item("timestamp", unix_seconds(metadata))?;
    dict.set_item("key_values", key_values_to_dict(py, &metadata.key_values)?)?;
    Ok(dict)
}

/// Read the record timestamp as fractional seconds since the Unix epoch.
///
/// A timestamp before the epoch has no sensible representation here, so it
/// falls back to zero rather than failing the whole record conversion.
fn unix_seconds(metadata: &RecordMetadata) -> f64 {
    metadata
        .timestamp
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

/// Copy a record's structured key/value pairs into a Python dict.
fn key_values_to_dict<'py>(
    py: Python<'py>,
    key_values: &BTreeMap<String, String>,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    for (key, value) in key_values {
        dict.set_item(key, value)?;
    }
    Ok(dict)
}

/// Attach the optional exception and stack payloads to `dict`.
fn attach_payloads(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    record: &FemtoLogRecord,
) -> PyResult<()> {
    if let Some(exc) = record.exception_payload() {
        dict.set_item("exc_info", exception_payload_to_py(py, exc)?)?;
    }
    if let Some(stack) = record.stack_payload() {
        dict.set_item("stack_info", stack_payload_to_py(py, stack)?)?;
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
    if let Some(ref locals) = frame.locals {
        let locals_dict = PyDict::new(py);
        for (k, v) in locals {
            locals_dict.set_item(k, v)?;
        }
        dict.set_item("locals", locals_dict)?;
    }

    Ok(dict.into())
}

/// Convert a `StackTracePayload` to a Python dict.
fn stack_payload_to_py(py: Python<'_>, payload: &StackTracePayload) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item("schema_version", payload.schema_version)?;

    let frames_list = PyList::empty(py);
    for frame in &payload.frames {
        frames_list.append(stack_frame_to_py(py, frame)?)?;
    }
    dict.set_item("frames", frames_list)?;

    Ok(dict.into())
}

/// Convert a `&[String]` to a Python list.
fn string_vec_to_pylist(py: Python<'_>, strings: &[String]) -> PyResult<Py<PyAny>> {
    let list = PyList::empty(py);
    for s in strings {
        list.append(s)?;
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
    for exc in exceptions {
        list.append(exception_payload_to_py(py, exc)?)?;
    }
    Ok(list.into())
}

/// Set optional fields on the exception payload dict.
///
/// The insertion order is observable: a Python caller may iterate the dict, so
/// the fields are written in the order the payload has always carried them.
/// Splitting the writes into helpers must not reorder them.
fn set_optional_exception_items(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    payload: &ExceptionPayload,
) -> PyResult<()> {
    set_exception_module_item(dict, payload)?;
    set_exception_sequence_items(py, dict, payload)?;
    set_exception_link_items(py, dict, payload)?;
    set_exception_suppression_item(dict, payload)?;
    // Written last so the payload dict keeps the key order it has always had;
    // callers would otherwise observe `exceptions` before the chained fields.
    set_exception_member_items(py, dict, payload)
}

/// Set the module name, when the exception class carries one.
fn set_exception_module_item(dict: &Bound<'_, PyDict>, payload: &ExceptionPayload) -> PyResult<()> {
    if let Some(ref module) = payload.module {
        dict.set_item("module", module)?;
    }
    Ok(())
}

/// Set the argument renderings, notes and stack frames, when present.
fn set_exception_sequence_items(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    payload: &ExceptionPayload,
) -> PyResult<()> {
    if !payload.args_repr.is_empty() {
        dict.set_item("args_repr", string_vec_to_pylist(py, &payload.args_repr)?)?;
    }
    if !payload.notes.is_empty() {
        dict.set_item("notes", string_vec_to_pylist(py, &payload.notes)?)?;
    }
    if !payload.frames.is_empty() {
        dict.set_item("frames", frames_to_pylist(py, &payload.frames)?)?;
    }
    Ok(())
}

/// Set the `__cause__` and `__context__` links to other payloads.
fn set_exception_link_items(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    payload: &ExceptionPayload,
) -> PyResult<()> {
    if let Some(ref cause) = payload.cause {
        dict.set_item("cause", exception_payload_to_py(py, cause)?)?;
    }
    if let Some(ref context) = payload.context {
        dict.set_item("context", exception_payload_to_py(py, context)?)?;
    }
    Ok(())
}

/// Record that the exception suppresses implicit context.
fn set_exception_suppression_item(
    dict: &Bound<'_, PyDict>,
    payload: &ExceptionPayload,
) -> PyResult<()> {
    if payload.suppress_context {
        dict.set_item("suppress_context", true)?;
    }
    Ok(())
}

/// Set the exception-group members, when the exception group has any.
fn set_exception_member_items(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    payload: &ExceptionPayload,
) -> PyResult<()> {
    if !payload.exceptions.is_empty() {
        dict.set_item("exceptions", exceptions_to_pylist(py, &payload.exceptions)?)?;
    }
    Ok(())
}

/// Convert an `ExceptionPayload` to a Python dict.
fn exception_payload_to_py(py: Python<'_>, payload: &ExceptionPayload) -> PyResult<Py<PyAny>> {
    let dict = PyDict::new(py);
    dict.set_item("schema_version", payload.schema_version)?;
    dict.set_item("type_name", &payload.type_name)?;
    dict.set_item("message", &payload.message)?;

    set_optional_exception_items(py, &dict, payload)?;

    Ok(dict.into())
}

impl FemtoFormatter for PythonFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        match self.call(record) {
            Ok(result) => result,
            Err(err) => Python::attach(|py| {
                err.print(py);
                format!("<formatter error in {}>", self.description)
            }),
        }
    }
}

/// Convert a Python formatter object into a shared [`FemtoFormatter`] (`Arc` trait object).
pub fn formatter_from_py(obj: &Bound<'_, PyAny>) -> PyResult<SharedFormatter> {
    PythonFormatter::try_new(obj)
        .map(|formatter| SharedFormatter::from_arc(Arc::new(formatter)))
        .map_err(|err| {
            let py = obj.py();
            let context = PyTypeError::new_err(
                "formatter must be callable or expose a format(record: Mapping) -> str method",
            );
            context.set_cause(py, Some(err));
            context
        })
}

#[cfg(test)]
#[path = "python_tests.rs"]
mod tests;
