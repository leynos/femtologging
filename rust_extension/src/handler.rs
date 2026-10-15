//! Core handler trait and error types shared by every handler
//! implementation.

use crate::log_record::FemtoLogRecord;
use pyo3::prelude::*;
use std::{any::Any, time::Duration};
use thiserror::Error;

/// Errors reported by handler implementations when dispatching a log record.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HandlerError {
    /// The handler's queue rejected the record because it was already full.
    #[error("queue full")]
    QueueFull,
    /// The handler is no longer accepting records because it has been closed.
    #[error("handler is closed")]
    Closed,
    /// A Python-backed handler did not opt in to contextual dispatch.
    #[error("Python-backed handlers must provide contextual dispatch")]
    MissingContextDispatch,
    /// Sending the record timed out before the handler could accept it.
    #[error("handler send timed out after {0:?}")]
    Timeout(Duration),
    /// Catch-all variant for handler specific failures.
    #[error("{0}")]
    Message(String),
}

impl From<HandlerError> for PyErr {
    fn from(error: HandlerError) -> Self {
        pyo3::exceptions::PyRuntimeError::new_err(error.to_string())
    }
}

/// Trait implemented by all log handlers.
///
/// `FemtoHandler` is `Send + Sync` so it can be safely called from multiple
/// threads by reference. Each implementation forwards the record to its own
/// consumer thread without blocking the caller.
pub trait FemtoHandlerTrait: Send + Sync + Any {
    /// Dispatch a log record for handling.
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError>;

    /// Return whether this handler invokes Python while dispatching records.
    ///
    /// Python-backed handlers must override [`Self::handle_with_context`] to
    /// use the supplied context and return `true` from
    /// [`Self::provides_context_dispatch`] to declare that capability.
    fn is_python_backed(&self) -> bool {
        false
    }

    /// Report whether this handler provides contextual Python dispatch.
    ///
    /// Python-backed implementations must return `true` only when they
    /// override [`Self::handle_with_context`] and invoke Python inside the
    /// supplied context.
    #[cfg(feature = "python")]
    fn provides_context_dispatch(&self) -> bool {
        false
    }

    /// Dispatch a record inside a captured Python context when applicable.
    ///
    /// The default preserves direct dispatch for native handlers. Python-backed
    /// handlers must override this method to invoke Python inside the supplied
    /// context and return `true` from [`Self::provides_context_dispatch`]; the
    /// default fails closed rather than silently discarding captured context.
    #[cfg(feature = "python")]
    fn handle_with_context(
        &self,
        record: FemtoLogRecord,
        _context: Option<&Py<PyAny>>,
    ) -> Result<(), HandlerError> {
        if self.is_python_backed() {
            Err(HandlerError::MissingContextDispatch)
        } else {
            self.handle(record)
        }
    }

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
