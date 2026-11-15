//! Python handler wrapper for FemtoLogger.
//!
//! This module provides [`PyHandler`], which wraps Python handler objects
//! to allow them to be used by the Rust logging infrastructure.

#[cfg(feature = "python")]
use pyo3::Py;
use pyo3::PyAny;
use pyo3::prelude::*;
#[cfg(feature = "python")]
use pyo3::types::PyAnyMethods;
#[cfg(feature = "python")]
use std::any::Any;

#[cfg(feature = "python")]
use crate::handler::FemtoHandlerTrait;
use crate::handler::HandlerError;
// Only the feature-enabled wrapper has a record to dispatch; the fallback in
// `no_python` brings its own import.
#[cfg(feature = "python")]
use crate::log_record::FemtoLogRecord;
use log::warn;

#[cfg(feature = "python")]
mod calls;
#[cfg(feature = "python")]
mod flush;
#[cfg(feature = "python")]
mod native_handler;
#[cfg(feature = "python")]
use self::native_handler::NativeHandlerKind;
#[cfg(feature = "python")]
mod method_call;

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

    /// Flush the wrapped handler so `flush_handlers` covers Python-registered
    /// handlers too.
    ///
    /// A recognized built-in handler is flushed through its native
    /// implementation, which reaches buffered records even when the Python
    /// wrapper exposes no `flush` method. Any other Python object is flushed
    /// through its own `flush` method when it has a callable one, so a
    /// user-defined Python handler controls its own buffer.
    fn flush(&self) -> bool {
        self.flush_registered_handler()
    }

    fn is_python_backed(&self) -> bool {
        true
    }

    fn provides_context_dispatch(&self) -> bool {
        true
    }

    /// Flush a wrapped handler under the barrier's deadline.
    ///
    /// A built-in handler keeps its own queue and buffer, so the barrier asks
    /// its native implementation to flush. Reporting success without doing so
    /// would confirm records that are still buffered.
    ///
    /// Any other Python object is asked to flush itself, exactly as the legacy
    /// [`Self::flush`] path does. A user-defined handler may buffer records
    /// after `handle` or `handle_record` returns, and only the handler knows
    /// how to drain that buffer, so the barrier calls its `flush` method. A
    /// handler without a callable `flush` has nothing to drain and confirms by
    /// opting out; one that raises, or returns a value that is falsy but not
    /// `None`, reports failure, because CPython's `None` is the usual "no
    /// result" answer from a successful flush. The exception itself is never
    /// printed, in case it quotes a record.
    ///
    /// The barrier's deadline bounds the wait for the handler to answer, not
    /// the duration of the handler's own `flush` call, which runs to
    /// completion on the shard worker.
    fn flush_until(&self, context: &crate::handler::FlushContext) -> bool {
        Python::attach(|py| match self.native_handler {
            Some(native_handler) => native_handler.flush_until(py, self.obj.bind(py), context),
            None => self.call_python_flush(py),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(all(test, feature = "python"))]
#[path = "py_handler/flush_diagnostics_tests.rs"]
mod flush_diagnostics_tests;

#[cfg(all(test, feature = "python"))]
#[path = "py_handler/tests.rs"]
mod tests;

#[cfg(not(feature = "python"))]
mod no_python;
#[cfg(not(feature = "python"))]
pub use self::no_python::PyHandler;
