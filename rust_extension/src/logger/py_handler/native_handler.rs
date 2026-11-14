//! Native dispatch for built-in handlers registered through Python.

use log::warn;
use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;
use pyo3::{Bound, PyAny, PyClass};

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::handlers::{
    rotating::PyRotatingFileHandler, timed_rotating::PyTimedRotatingFileHandler,
};
use crate::log_record::FemtoLogRecord;
use crate::{FemtoFileHandler, FemtoHTTPHandler, FemtoSocketHandler, FemtoStreamHandler};

use super::map_py_err;

/// Built-in handlers that can retain the original Rust record across Python registration.
#[derive(Clone, Copy)]
pub(super) enum NativeHandlerKind {
    /// Retains native record dispatch for a stream handler wrapped for Python.
    Stream,
    /// Retains native record dispatch for a file handler wrapped for Python.
    File,
    /// Retains native record dispatch for a rotating file handler wrapped for Python.
    RotatingFile,
    /// Retains native record dispatch for a timed-rotating file handler wrapped for Python.
    TimedRotatingFile,
    /// Retains native record dispatch for a socket handler wrapped for Python.
    Socket,
    /// Retains native record dispatch for an HTTP handler wrapped for Python.
    Http,
}

impl NativeHandlerKind {
    /// Identify a built-in Python handler whose native implementation accepts
    /// the complete Rust record.
    ///
    /// Returns `None` for user-defined handlers, which use the Python handler
    /// interface instead.
    pub(super) fn from_object(obj: &Bound<'_, PyAny>) -> Option<Self> {
        if obj.is_instance_of::<FemtoStreamHandler>() {
            Some(Self::Stream)
        } else if obj.is_instance_of::<FemtoFileHandler>() {
            Some(Self::File)
        } else if obj.is_instance_of::<PyRotatingFileHandler>() {
            Some(Self::RotatingFile)
        } else if obj.is_instance_of::<PyTimedRotatingFileHandler>() {
            Some(Self::TimedRotatingFile)
        } else if obj.is_instance_of::<FemtoSocketHandler>() {
            Some(Self::Socket)
        } else if obj.is_instance_of::<FemtoHTTPHandler>() {
            Some(Self::Http)
        } else {
            None
        }
    }

    /// Flush the wrapped built-in handler, releasing the GIL while it waits.
    ///
    /// The handler's own Python `flush` method releases the GIL for the same
    /// reason: a Python callable formatter needs it on the worker thread
    /// before the flush can be acknowledged.
    pub(super) fn flush(self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> bool {
        match self {
            Self::Stream => flush_builtin::<FemtoStreamHandler>(py, obj),
            Self::File => flush_builtin::<FemtoFileHandler>(py, obj),
            Self::RotatingFile => flush_builtin::<PyRotatingFileHandler>(py, obj),
            Self::TimedRotatingFile => flush_builtin::<PyTimedRotatingFileHandler>(py, obj),
            Self::Socket => flush_builtin::<FemtoSocketHandler>(py, obj),
            Self::Http => flush_builtin::<FemtoHTTPHandler>(py, obj),
        }
    }

    /// Dispatch the record through the matching built-in handler while
    /// retaining its metadata, including structured key/value fields.
    ///
    /// Returns a handler error if extracting the wrapped native handler or
    /// delivering the record fails.
    pub(super) fn handle(
        self,
        py: Python<'_>,
        obj: &Bound<'_, PyAny>,
        record: FemtoLogRecord,
    ) -> Result<(), HandlerError> {
        match self {
            Self::Stream => obj
                .extract::<PyRef<'_, FemtoStreamHandler>>()
                .map_err(|err| map_py_err(py, err.into(), "extract FemtoStreamHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
            Self::File => obj
                .extract::<PyRef<'_, FemtoFileHandler>>()
                .map_err(|err| map_py_err(py, err.into(), "extract FemtoFileHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
            Self::RotatingFile => obj
                .extract::<PyRef<'_, PyRotatingFileHandler>>()
                .map_err(|err| map_py_err(py, err.into(), "extract FemtoRotatingFileHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
            Self::TimedRotatingFile => obj
                .extract::<PyRef<'_, PyTimedRotatingFileHandler>>()
                .map_err(|err| map_py_err(py, err.into(), "extract FemtoTimedRotatingFileHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
            Self::Socket => obj
                .extract::<PyRef<'_, FemtoSocketHandler>>()
                .map_err(|err| map_py_err(py, err.into(), "extract FemtoSocketHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
            Self::Http => obj
                .extract::<PyRef<'_, FemtoHTTPHandler>>()
                .map_err(|err| map_py_err(py, err.into(), "extract FemtoHTTPHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
        }
    }
}

/// Flush one built-in handler with the interpreter detached for the wait.
///
/// Extraction needs the GIL, but the wait must not hold it: a built-in whose
/// formatter is a Python callable needs the GIL on its worker thread before
/// it can acknowledge. `T` is `Sync`, so the shared borrow is safe to carry
/// into the detached closure.
fn flush_builtin<T>(py: Python<'_>, obj: &Bound<'_, PyAny>) -> bool
where
    T: FemtoHandlerTrait + PyClass + Sync,
{
    match obj.extract::<PyRef<'_, T>>() {
        Ok(handler) => {
            let handler: &T = &handler;
            let flushed = py.detach(|| handler.flush());
            if !flushed {
                // `flush_handlers()` newly reports these failures, so an
                // operator needs a bounded record of the handler failing to
                // flush rather than only a `False` return.
                warn!("PyHandler: flush failed: handler_kind=native, category=flush_failed");
            }
            flushed
        }
        Err(err) => {
            map_py_err(py, err.into(), "extract handler for flush");
            false
        }
    }
}
