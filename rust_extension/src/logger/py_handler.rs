//! Python handler wrapper for FemtoLogger.
//!
//! This module provides [`PyHandler`], which wraps Python handler objects
//! to allow them to be used by the Rust logging infrastructure.

use pyo3::prelude::*;
#[cfg(feature = "python")]
use pyo3::types::PyAnyMethods;
use pyo3::{Py, PyAny};
use std::any::Any;

#[cfg(feature = "python")]
use crate::formatter::python::record_to_dict;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;
use log::warn;

#[cfg(feature = "python")]
mod native_handler;
#[cfg(feature = "python")]
use self::native_handler::NativeHandlerKind;

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
    /// Built-in Rust handler wrapped by a Python object, if applicable.
    native_handler: Option<NativeHandlerKind>,
}

#[cfg(feature = "python")]
struct DirectMethodSpec {
    name: &'static str,
    lookup_operation: &'static str,
    call_operation: &'static str,
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
    ///
    /// # Handler stability contract
    ///
    /// Handler capabilities are inspected **once** at construction time and
    /// cached for the lifetime of the `PyHandler`. Specifically, the presence
    /// of a callable `handle_record` method is checked when `PyHandler::new`
    /// is called and determines which dispatch path is used for all
    /// subsequent log records.
    ///
    /// **Mutating the handler object after registration results in undefined
    /// behaviour.** If `handle_record` is added to or removed from the
    /// underlying Python object after construction, the cached capability
    /// will not reflect the change and the wrong method may be invoked.
    ///
    /// This design mirrors [`validate_handler`], which also performs a
    /// one-time check at registration rather than on every log record, and
    /// aligns with the standard library `logging.Handler` expectation that
    /// handlers are fully configured before use.
    pub fn new(py: Python<'_>, obj: Py<PyAny>) -> Self {
        let native_handler = NativeHandlerKind::from_object(obj.bind(py));
        let has_handle_record = obj
            .getattr(py, "handle_record")
            .map(|attr| attr.bind(py).is_callable())
            .unwrap_or(false);
        Self {
            obj,
            has_handle_record,
            native_handler,
        }
    }

    /// Call the structured `handle_record` method with the full record dict.
    fn call_handle_record(
        &self,
        py: Python<'_>,
        record: &FemtoLogRecord,
        context: Option<&Py<PyAny>>,
    ) -> Result<(), HandlerError> {
        let record_dict =
            record_to_dict(py, record).map_err(|err| map_py_err(py, err, "record_to_dict"))?;
        match context {
            Some(context) => self.call_handle_record_in_context(py, context, record_dict),
            None => self.call_handle_record_direct(py, record_dict),
        }
    }

    fn call_handle_record_in_context(
        &self,
        py: Python<'_>,
        context: &Py<PyAny>,
        record_dict: Py<PyAny>,
    ) -> Result<(), HandlerError> {
        let method_caller = py
            .import("operator")
            .and_then(|operator| operator.getattr("methodcaller"))
            .and_then(|factory| factory.call1(("handle_record", record_dict)))
            .map_err(|err| map_py_err(py, err, "create handle_record caller"))?;
        self.call_methodcaller_in_context(py, context, method_caller, "handle_record")
    }

    fn call_handle_record_direct(
        &self,
        py: Python<'_>,
        record_dict: Py<PyAny>,
    ) -> Result<(), HandlerError> {
        self.call_direct_method(
            py,
            DirectMethodSpec {
                name: "handle_record",
                lookup_operation: "get handle_record",
                call_operation: "handle_record",
            },
            (record_dict,),
        )
    }

    /// Call the legacy 3-argument `handle` method.
    fn call_legacy_handle(
        &self,
        py: Python<'_>,
        record: &FemtoLogRecord,
        context: Option<&Py<PyAny>>,
    ) -> Result<(), HandlerError> {
        match context {
            Some(context) => self.call_legacy_handle_in_context(py, context, record),
            None => self.call_legacy_handle_direct(py, record),
        }
    }

    fn call_legacy_handle_in_context(
        &self,
        py: Python<'_>,
        context: &Py<PyAny>,
        record: &FemtoLogRecord,
    ) -> Result<(), HandlerError> {
        let method_caller = py
            .import("operator")
            .and_then(|operator| operator.getattr("methodcaller"))
            .and_then(|factory| {
                factory.call1((
                    "handle",
                    record.logger(),
                    record.level_str(),
                    record.message(),
                ))
            })
            .map_err(|err| map_py_err(py, err, "create handle caller"))?;
        self.call_methodcaller_in_context(py, context, method_caller, "handle")
    }

    fn call_legacy_handle_direct(
        &self,
        py: Python<'_>,
        record: &FemtoLogRecord,
    ) -> Result<(), HandlerError> {
        self.call_direct_method(
            py,
            DirectMethodSpec {
                name: "handle",
                lookup_operation: "get handle",
                call_operation: "handle",
            },
            (record.logger(), record.level_str(), record.message()),
        )
    }

    fn call_direct_method<'py>(
        &self,
        py: Python<'py>,
        method: DirectMethodSpec,
        args: impl pyo3::call::PyCallArgs<'py>,
    ) -> Result<(), HandlerError> {
        let handler_method = self
            .obj
            .bind(py)
            .getattr(method.name)
            .map_err(|err| map_py_err(py, err, method.lookup_operation))?;
        handler_method
            .call1(args)
            .map(|_| ())
            .map_err(|err| map_py_err(py, err, method.call_operation))
    }

    fn call_methodcaller_in_context<'py>(
        &self,
        py: Python<'py>,
        context: &Py<PyAny>,
        method_caller: Bound<'py, PyAny>,
        method: &str,
    ) -> Result<(), HandlerError> {
        let invocation_context = context
            .bind(py)
            .call_method0("copy")
            .map_err(|err| map_py_err(py, err, "copy context"))?;
        invocation_context
            .call_method1("run", (method_caller, self.obj.bind(py)))
            .map(|_| ())
            .map_err(|err| map_py_err(py, err, method))
    }
}

#[cfg(feature = "python")]
impl FemtoHandlerTrait for PyHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.handle_with_context(record, None)
    }

    /// Python handlers run inside the producer's captured context when given.
    fn handle_with_context(
        &self,
        record: FemtoLogRecord,
        context: Option<&Py<PyAny>>,
    ) -> Result<(), HandlerError> {
        Python::attach(|py| {
            if let Some(native_handler) = self.native_handler {
                return native_handler.handle(py, self.obj.bind(py), record);
            }
            if self.has_handle_record {
                return self.call_handle_record(py, &record, context);
            }
            self.call_legacy_handle(py, &record, context)
        })
    }

    fn is_python_backed(&self) -> bool {
        true
    }

    fn provides_context_dispatch(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(all(test, feature = "python"))]
mod tests {
    //! Tests for native handler recognition in the Python wrapper.

    use super::NativeHandlerKind;
    use crate::{FemtoHTTPHandler, FemtoSocketHandler, HTTPHandlerConfig, SocketHandlerConfig};
    use pyo3::{Py, Python};

    /// Built-in socket and HTTP handlers are recognized for native dispatch.
    #[test]
    fn native_handler_kind_recognizes_network_handlers() {
        Python::attach(|py| {
            let socket = Py::new(
                py,
                FemtoSocketHandler::with_config(SocketHandlerConfig::default()),
            )
            .expect("socket handler should be constructible as a Python object");
            assert!(matches!(
                NativeHandlerKind::from_object(socket.bind(py)),
                Some(NativeHandlerKind::Socket)
            ));

            let http = Py::new(
                py,
                FemtoHTTPHandler::with_config(HTTPHandlerConfig::default()),
            )
            .expect("HTTP handler should be constructible as a Python object");
            assert!(matches!(
                NativeHandlerKind::from_object(http.bind(py)),
                Some(NativeHandlerKind::Http)
            ));
        });
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
        Python::attach(|py| {
            self.obj
                .call_method1(
                    py,
                    "handle",
                    (record.logger(), record.level_str(), record.message()),
                )
                .map(|_| ())
                .map_err(|err| map_py_err(py, err, "handle"))
        })
    }

    fn is_python_backed(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
