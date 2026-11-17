//! Helpers for adapting Python callables into [`FemtoFormatter`] instances.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

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
    populate_record_scalars(&dict, record)?;
    dict.set_item("metadata", build_metadata_dict(py, record)?)?;
    add_exception_payload(py, &dict, record)?;
    add_stack_payload(py, &dict, record)?;
    Ok(dict.into())
}

/// Populate the top-level logger fields shared by every consumer.
fn populate_record_scalars(dict: &Bound<'_, PyDict>, record: &FemtoLogRecord) -> PyResult<()> {
    dict.set_item("logger", record.logger())?;
    dict.set_item("level", record.level_str())?;
    dict.set_item("message", record.message())?;
    dict.set_item("levelno", u8::from(record.level()))?;
    Ok(())
}

/// Build the nested metadata mapping, including structured key-values.
fn build_metadata_dict<'py>(
    py: Python<'py>,
    record: &FemtoLogRecord,
) -> PyResult<Bound<'py, PyDict>> {
    let rec_metadata = record.metadata();
    let metadata = PyDict::new(py);
    populate_metadata_scalars(&metadata, rec_metadata)?;
    metadata.set_item("key_values", key_values_dict(py, &rec_metadata.key_values)?)?;
    Ok(metadata)
}

/// Populate the scalar metadata fields.
fn populate_metadata_scalars(
    metadata: &Bound<'_, PyDict>,
    rec_metadata: &RecordMetadata,
) -> PyResult<()> {
    metadata.set_item("module_path", &rec_metadata.module_path)?;
    metadata.set_item("filename", &rec_metadata.filename)?;
    metadata.set_item("line_number", rec_metadata.line_number)?;
    metadata.set_item("thread_name", &rec_metadata.thread_name)?;
    metadata.set_item("thread_id", format!("{:?}", rec_metadata.thread_id))?;
    metadata.set_item("timestamp", timestamp_seconds(rec_metadata.timestamp))?;
    Ok(())
}

/// Convert a record timestamp to fractional seconds since the Unix epoch.
fn timestamp_seconds(timestamp: SystemTime) -> f64 {
    timestamp
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or_default()
}

/// Build the structured key-value mapping.
fn key_values_dict<'py>(
    py: Python<'py>,
    key_values: &BTreeMap<String, String>,
) -> PyResult<Bound<'py, PyDict>> {
    let kv = PyDict::new(py);
    for (key, value) in key_values {
        kv.set_item(key, value)?;
    }
    Ok(kv)
}

/// Add the exception payload when the record carries one.
fn add_exception_payload(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    record: &FemtoLogRecord,
) -> PyResult<()> {
    let Some(exc) = record.exception_payload() else {
        return Ok(());
    };
    dict.set_item("exc_info", exception_payload_to_py(py, exc)?)
}

/// Add the stack payload when the record carries one.
fn add_stack_payload(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    record: &FemtoLogRecord,
) -> PyResult<()> {
    let Some(stack) = record.stack_payload() else {
        return Ok(());
    };
    dict.set_item("stack_info", stack_payload_to_py(py, stack)?)
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
fn set_optional_exception_items(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    payload: &ExceptionPayload,
) -> PyResult<()> {
    set_exception_module(dict, payload)?;
    set_string_list_item(py, dict, "args_repr", &payload.args_repr)?;
    set_string_list_item(py, dict, "notes", &payload.notes)?;
    set_exception_frames(py, dict, payload)?;
    set_nested_payload_item(py, dict, "cause", payload.cause.as_deref())?;
    set_nested_payload_item(py, dict, "context", payload.context.as_deref())?;
    set_exception_suppress_context(dict, payload)?;
    set_exception_children(py, dict, payload)
}

/// Set the optional `module` field.
fn set_exception_module(dict: &Bound<'_, PyDict>, payload: &ExceptionPayload) -> PyResult<()> {
    if let Some(module) = &payload.module {
        dict.set_item("module", module)?;
    }
    Ok(())
}

/// Set a non-empty string list field.
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

/// Set the optional `frames` list.
fn set_exception_frames(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    payload: &ExceptionPayload,
) -> PyResult<()> {
    if !payload.frames.is_empty() {
        dict.set_item("frames", frames_to_pylist(py, &payload.frames)?)?;
    }
    Ok(())
}

/// Set an optional nested exception payload field.
fn set_nested_payload_item(
    py: Python<'_>,
    dict: &Bound<'_, PyDict>,
    key: &str,
    payload: Option<&ExceptionPayload>,
) -> PyResult<()> {
    if let Some(payload) = payload {
        dict.set_item(key, exception_payload_to_py(py, payload)?)?;
    }
    Ok(())
}

/// Set the optional `suppress_context` flag.
fn set_exception_suppress_context(
    dict: &Bound<'_, PyDict>,
    payload: &ExceptionPayload,
) -> PyResult<()> {
    if payload.suppress_context {
        dict.set_item("suppress_context", true)?;
    }
    Ok(())
}

/// Set the optional nested exception group members.
fn set_exception_children(
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
mod tests {
    //! Tests for record conversion helpers.

    use std::collections::BTreeMap;
    use std::time::{Duration, UNIX_EPOCH};

    use pyo3::Python;
    use pyo3::types::{PyAnyMethods, PyDictMethods};

    use super::{key_values_dict, timestamp_seconds};

    #[test]
    fn timestamp_seconds_treats_pre_epoch_times_as_zero() {
        assert_eq!(timestamp_seconds(UNIX_EPOCH - Duration::from_secs(1)), 0.0);
    }

    #[test]
    fn key_values_dict_copies_structured_values() {
        Python::attach(|py| {
            let values = BTreeMap::from([("request_id".to_owned(), "abc".to_owned())]);
            let dict = key_values_dict(py, &values).expect("build key-value dict");
            let request_id: String = dict
                .get_item("request_id")
                .expect("lookup")
                .expect("present")
                .extract()
                .expect("string value");
            assert_eq!(request_id, "abc");
        });
    }
}
