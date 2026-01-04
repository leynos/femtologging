//! Python handler wrapper for FemtoLogger.
//!
//! This module provides [`PyHandler`], which wraps Python handler objects
//! to allow them to be used by the Rust logging infrastructure.

use pyo3::prelude::*;
use pyo3::{Py, PyAny};
use std::any::Any;

#[cfg(feature = "python")]
use crate::formatter::python::record_to_dict;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;
use log::warn;

/// Map a Python error to a [`HandlerError`], logging a warning.
fn map_py_err(py: Python<'_>, err: PyErr, method: &str) -> HandlerError {
    let message = err.to_string();
    err.print(py);
    warn!("PyHandler: error calling {method}: {message}");
    HandlerError::Message(format!("python handler raised an exception: {message}"))
}

/// Validate that a Python object has a callable `handle` method.
///
/// This function checks whether the provided Python object is suitable for use
/// as a handler by verifying it has a `handle` attribute that is callable.
///
/// # Parameters
///
/// * `obj` - A reference to the Python object to validate.
///
/// # Returns
///
/// * `Ok(())` if the object has a callable `handle` method.
/// * `Err(PyTypeError)` if the `handle` attribute is missing or not callable.
///
/// # Errors
///
/// Returns a `PyTypeError` in the following cases:
/// - The object has no `handle` attribute (message: "handler must implement a
///   callable 'handle' method")
/// - The `handle` attribute exists but is not callable (message includes the
///   attribute type and handler representation)
pub fn validate_handler(obj: &Bound<'_, PyAny>) -> PyResult<()> {
    let py = obj.py();
    let handle = obj.getattr("handle").map_err(|err| {
        if err.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) {
            pyo3::exceptions::PyTypeError::new_err(
                "handler must implement a callable 'handle' method",
            )
        } else {
            err
        }
    })?;
    if handle.is_callable() {
        Ok(())
    } else {
        let attr_type = handle
            .get_type()
            .name()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "<unknown>".to_string());
        let handler_repr = obj
            .repr()
            .map(|r| r.to_string())
            .unwrap_or_else(|_| "<unrepresentable>".to_string());
        Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "'handler.handle' is not callable (type: {attr_type}, handler: {handler_repr})",
        )))
    }
}

/// Wrapper allowing Python handler objects to be used by the logger.
///
/// `PyHandler` bridges the Rust logging infrastructure with Python handler
/// objects by implementing [`FemtoHandlerTrait`]. It supports two handler
/// interfaces:
///
/// 1. **Structured interface** (`handle_record`): If the Python handler has a
///    callable `handle_record` method, the full log record is passed as a
///    dictionary, providing access to all structured fields.
///
/// 2. **Legacy interface** (`handle`): The handler's `handle` method is called
///    with three positional arguments: logger name, level, and message.
///
/// The structured interface is preferred when available, falling back to the
/// legacy interface otherwise.
#[cfg(feature = "python")]
pub struct PyHandler {
    /// The underlying Python handler object.
    pub obj: Py<PyAny>,
    /// Whether this handler has a `handle_record` method for structured payloads.
    has_handle_record: bool,
}

#[cfg(feature = "python")]
impl PyHandler {
    /// Create a new `PyHandler` from a Python object.
    ///
    /// Inspects the Python object to determine whether it has a callable
    /// `handle_record` method. If present, the structured interface will be
    /// used when handling log records; otherwise, the legacy `handle` method
    /// will be called.
    ///
    /// # Parameters
    ///
    /// * `py` - The Python interpreter token.
    /// * `obj` - The Python handler object to wrap. Should have at least a
    ///   callable `handle` method (validated separately via [`validate_handler`]).
    ///
    /// # Returns
    ///
    /// A new `PyHandler` instance wrapping the provided object.
    pub fn new(py: Python<'_>, obj: Py<PyAny>) -> Self {
        let has_handle_record = obj
            .getattr(py, "handle_record")
            .map(|attr| attr.bind(py).is_callable())
            .unwrap_or(false);
        Self {
            obj,
            has_handle_record,
        }
    }

    /// Call the structured `handle_record` method with the full record dict.
    fn call_handle_record(
        &self,
        py: Python<'_>,
        record: &FemtoLogRecord,
    ) -> Result<(), HandlerError> {
        let record_dict =
            record_to_dict(py, record).map_err(|err| map_py_err(py, err, "record_to_dict"))?;

        self.obj
            .call_method1(py, "handle_record", (record_dict,))
            .map(|_| ())
            .map_err(|err| map_py_err(py, err, "handle_record"))
    }

    /// Call the legacy 3-argument `handle` method.
    fn call_legacy_handle(
        &self,
        py: Python<'_>,
        record: &FemtoLogRecord,
    ) -> Result<(), HandlerError> {
        self.obj
            .call_method1(
                py,
                "handle",
                (&record.logger, record.level.as_str(), &record.message),
            )
            .map(|_| ())
            .map_err(|err| map_py_err(py, err, "handle"))
    }
}

#[cfg(feature = "python")]
impl FemtoHandlerTrait for PyHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        Python::with_gil(|py| {
            if self.has_handle_record {
                return self.call_handle_record(py, &record);
            }
            self.call_legacy_handle(py, &record)
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Fallback PyHandler when python feature is disabled.
#[cfg(not(feature = "python"))]
pub struct PyHandler {
    pub obj: Py<PyAny>,
}

#[cfg(not(feature = "python"))]
impl PyHandler {
    pub fn new(_py: Python<'_>, obj: Py<PyAny>) -> Self {
        Self { obj }
    }
}

#[cfg(not(feature = "python"))]
impl FemtoHandlerTrait for PyHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        Python::with_gil(|py| {
            self.obj
                .call_method1(
                    py,
                    "handle",
                    (&record.logger, record.level.as_str(), &record.message),
                )
                .map(|_| ())
                .map_err(|err| map_py_err(py, err, "handle"))
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
