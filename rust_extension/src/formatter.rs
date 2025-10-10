//! Formatter implementations and adapters bridging Rust and Python callers.
//!
//! Provides the core [`FemtoFormatter`] trait alongside helpers for
//! dynamically dispatched trait objects. When the Python feature is enabled we
//! expose adapters for Python callables so they can participate in Rust
//! logging pipelines safely across threads.

use std::sync::Arc;

use crate::log_record::FemtoLogRecord;

/// Trait for formatting log records into strings.
///
/// Implementors must be thread-safe (`Send + Sync`) so formatters can be
/// shared across threads in a logging system.
pub trait FemtoFormatter: Send + Sync {
    /// Format a log record into a string representation.
    fn format(&self, record: &FemtoLogRecord) -> String;
}

/// Shared formatter trait object used across handlers.
pub type SharedFormatter = Arc<dyn FemtoFormatter + Send + Sync>;

#[derive(Copy, Clone, Debug)]
pub struct DefaultFormatter;

impl FemtoFormatter for DefaultFormatter {
    fn format(&self, record: &FemtoLogRecord) -> String {
        format!("{} [{}] {}", record.logger, record.level, record.message)
    }
}

impl FemtoFormatter for Arc<dyn FemtoFormatter + Send + Sync> {
    fn format(&self, record: &FemtoLogRecord) -> String {
        (**self).format(record)
    }
}

impl FemtoFormatter for Box<dyn FemtoFormatter + Send + Sync> {
    fn format(&self, record: &FemtoLogRecord) -> String {
        (**self).format(record)
    }
}

#[cfg(feature = "python")]
pub mod python {
    //! Helpers for adapting Python callables into [`FemtoFormatter`] instances.
    use std::sync::{Arc, Mutex};
    use std::time::UNIX_EPOCH;

    use pyo3::{
        exceptions::PyTypeError,
        prelude::*,
        types::{PyDict, PyString},
    };

    use crate::{log_record::FemtoLogRecord, python::fq_py_type};

    use super::{FemtoFormatter, SharedFormatter};

    #[derive(Clone)]
    struct PythonFormatter {
        callable: Arc<Mutex<Py<PyAny>>>,
        description: String,
    }

    impl PythonFormatter {
        fn try_new(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
            let description = fq_py_type(obj);
            if let Ok(s) = obj.downcast::<PyString>() {
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

        fn call(&self, record: &FemtoLogRecord) -> PyResult<String> {
            Python::with_gil(|py| {
                let payload = Self::record_to_dict(py, record)?;
                let callable = {
                    let guard = self
                        .callable
                        .lock()
                        .expect("Python formatter mutex must not be poisoned");
                    guard.clone_ref(py)
                };
                let result = callable.call1(py, (payload,))?;
                result.extract::<String>(py)
            })
        }

        fn record_to_dict(py: Python<'_>, record: &FemtoLogRecord) -> PyResult<PyObject> {
            let dict = PyDict::new(py);
            dict.set_item("logger", &record.logger)?;
            dict.set_item("level", &record.level)?;
            dict.set_item("message", &record.message)?;
            if let Some(level) = record.parsed_level {
                dict.set_item("levelno", u8::from(level))?;
            }

            let metadata = PyDict::new(py);
            metadata.set_item("module_path", &record.metadata.module_path)?;
            metadata.set_item("filename", &record.metadata.filename)?;
            metadata.set_item("line_number", record.metadata.line_number)?;
            metadata.set_item("thread_name", &record.metadata.thread_name)?;
            metadata.set_item("thread_id", format!("{:?}", record.metadata.thread_id))?;
            let timestamp = record
                .metadata
                .timestamp
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or_default();
            metadata.set_item("timestamp", timestamp)?;

            let kv = PyDict::new(py);
            for (key, value) in &record.metadata.key_values {
                kv.set_item(key, value)?;
            }
            metadata.set_item("key_values", kv)?;
            dict.set_item("metadata", metadata)?;

            Ok(dict.into())
        }
    }

    impl FemtoFormatter for PythonFormatter {
        fn format(&self, record: &FemtoLogRecord) -> String {
            match self.call(record) {
                Ok(result) => result,
                Err(err) => Python::with_gil(|py| {
                    err.print(py);
                    format!("<formatter error in {}>", self.description)
                }),
            }
        }
    }

    /// Convert a Python formatter object into a shared [`FemtoFormatter`] (`Arc` trait object).
    pub fn formatter_from_py(obj: &Bound<'_, PyAny>) -> PyResult<SharedFormatter> {
        PythonFormatter::try_new(obj)
            .map(|formatter| Arc::new(formatter) as SharedFormatter)
            .map_err(|err| {
                let py = obj.py();
                let context = PyTypeError::new_err(
                    "formatter must be callable or expose a format(record: Mapping) -> str method",
                );
                context.set_cause(py, Some(err));
                context
            })
    }
}
