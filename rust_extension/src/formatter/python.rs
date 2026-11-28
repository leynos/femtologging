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
    types::{PyDict, PyString},
};

use crate::log_record::{FemtoLogRecord, RecordMetadata};
use crate::python::fq_py_type;

use self::payload::{exception_payload_to_py, stack_payload_to_py};
use super::{FemtoFormatter, SharedFormatter};

mod payload;

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
