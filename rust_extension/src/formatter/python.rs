//! Convert log records to Python dictionaries, and Python callables to
//! [`FemtoFormatter`] instances.
//!
//! `record_to_dict` renders a [`FemtoLogRecord`] and its optional exception and
//! stack payloads as the mapping Python handlers consume, and
//! `formatter_from_py` wraps a Python callable as a [`FemtoFormatter`]. The
//! filters module builds the mapping a callback filter sees from the same
//! `record_to_dict` (`filters::python_callback::record_view`), and the Python
//! handler call path uses it too, so every Python-facing surface agrees on
//! field names and enrichment keys.

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
    dict.set_item("metadata", metadata_to_py(py, record.metadata())?)?;
    set_optional_payload_items(py, &dict, record)?;
    Ok(dict.into())
}

/// Build the record metadata dict, including structured key/value enrichment.
fn metadata_to_py<'py>(
    py: Python<'py>,
    rec_metadata: &RecordMetadata,
) -> PyResult<Bound<'py, PyDict>> {
    let metadata = PyDict::new(py);
    set_metadata_identity_items(&metadata, rec_metadata)?;
    set_metadata_time_items(py, &metadata, rec_metadata)?;
    Ok(metadata)
}

/// Set where the record was logged from and which thread logged it.
fn set_metadata_identity_items(
    metadata: &Bound<'_, PyDict>,
    rec_metadata: &RecordMetadata,
) -> PyResult<()> {
    metadata.set_item("module_path", &rec_metadata.module_path)?;
    metadata.set_item("filename", &rec_metadata.filename)?;
    metadata.set_item("line_number", rec_metadata.line_number)?;
    metadata.set_item("thread_name", &rec_metadata.thread_name)?;
    metadata.set_item("thread_id", format!("{:?}", rec_metadata.thread_id))?;
    Ok(())
}

/// Set when the record was created and the enrichment attached to it.
fn set_metadata_time_items(
    py: Python<'_>,
    metadata: &Bound<'_, PyDict>,
    rec_metadata: &RecordMetadata,
) -> PyResult<()> {
    metadata.set_item("timestamp", timestamp_seconds(rec_metadata))?;
    metadata.set_item("key_values", key_values_to_py(py, rec_metadata)?)?;
    Ok(())
}

/// Seconds since the Unix epoch; pre-epoch timestamps clamp to zero rather
/// than panicking inside the logging pipeline.
fn timestamp_seconds(rec_metadata: &RecordMetadata) -> f64 {
    rec_metadata
        .timestamp
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

/// Copy the record's structured key/value enrichment into a new dict.
fn key_values_to_py<'py>(
    py: Python<'py>,
    rec_metadata: &RecordMetadata,
) -> PyResult<Bound<'py, PyDict>> {
    let kv = PyDict::new(py);
    for (key, value) in &rec_metadata.key_values {
        kv.set_item(key, value)?;
    }
    Ok(kv)
}

/// Add the exception and stack payloads when the record carries them.
fn set_optional_payload_items(
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
