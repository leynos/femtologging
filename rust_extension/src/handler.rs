//! Handler traits and error conversion helpers shared by the logging
//! backends.
//!
//! Each handler implements [`FemtoHandlerTrait`] to accept formatted records
//! from the logger. Failures surface as [`HandlerError`] values, which can be
//! mapped to Python `RuntimeError`s using [`handler_error_to_py`]. The helper
//! keeps the Rust/Python boundary behaviour consistent by producing the same
//! [`PyErr`] message for every handler failure.
use crate::log_record::FemtoLogRecord;
use pyo3::{exceptions::PyRuntimeError, prelude::*, PyErr};
use std::{any::Any, time::Duration};
use thiserror::Error;

/// Errors reported by handler implementations when dispatching a log record.
#[non_exhaustive]
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HandlerError {
    /// The handler's queue rejected the record because it was already full.
    #[error("queue full")]
    QueueFull,
    /// The handler is no longer accepting records because it has been closed.
    #[error("handler is closed")]
    Closed,
    /// Sending the record timed out before the handler could accept it.
    #[error("handler send timed out after {0:?}")]
    Timeout(Duration),
    /// Catch-all variant for handler specific failures.
    #[error("{0}")]
    Message(String),
}

/// Convert a [`HandlerError`] into the Python exception exposed at the FFI
/// boundary.
pub(crate) fn handler_error_to_py(err: HandlerError) -> PyErr {
    PyRuntimeError::new_err(format!("Handler error: {err}"))
}

/// Trait implemented by all log handlers.
///
/// `FemtoHandler` is `Send + Sync` so it can be safely called from multiple
/// threads by reference. Each implementation forwards the record to its own
/// consumer thread without blocking the caller.
pub trait FemtoHandlerTrait: Send + Sync + Any {
    /// Dispatch a log record for handling.
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError>;

    /// Flush any pending log records.
    ///
    /// Returning `true` signals the flush completed successfully. Implementations
    /// may return `false` when the handler has been closed or if the flush
    /// command could not be processed.
    fn flush(&self) -> bool {
        // Default to a no-op flush for handlers that do not buffer writes.
        true
    }

    /// Expose a typed reference for downcasting.
    fn as_any(&self) -> &dyn Any;
}

/// Base Python class for handlers. Methods do nothing by default.
#[pyclass(name = "FemtoHandler", subclass)]
#[derive(Default)]
pub struct FemtoHandler;

#[pymethods]
impl FemtoHandler {
    #[new]
    fn py_new() -> Self {
        Self
    }
}

impl FemtoHandlerTrait for FemtoHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
