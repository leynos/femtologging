//! Native dispatch for built-in handlers registered through Python.

use pyo3::PyClass;
use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;
use pyo3::{Bound, PyAny};

use crate::handler::{FemtoHandlerTrait, FlushContext, HandlerError};
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

    /// Flush the matching built-in handler under the supplied deadline.
    ///
    /// A Python registration must not report a confirmed barrier while the
    /// wrapped handler still buffers records, so the handler's own
    /// deadline-aware flush runs here. Failing to extract the wrapped handler
    /// returns `false`: the barrier cannot confirm a flush it did not perform.
    pub(super) fn flush_until(
        self,
        py: Python<'_>,
        obj: &Bound<'_, PyAny>,
        context: &FlushContext,
    ) -> bool {
        let confirmed = match self {
            Self::Stream => flush_detached::<FemtoStreamHandler>(py, obj, context),
            Self::File => flush_detached::<FemtoFileHandler>(py, obj, context),
            Self::RotatingFile => flush_detached::<PyRotatingFileHandler>(py, obj, context),
            Self::TimedRotatingFile => {
                flush_detached::<PyTimedRotatingFileHandler>(py, obj, context)
            }
            Self::Socket => flush_detached::<FemtoSocketHandler>(py, obj, context),
            Self::Http => flush_detached::<FemtoHTTPHandler>(py, obj, context),
        };
        match confirmed {
            Ok(confirmed) => confirmed,
            Err(err) => {
                let _ = map_py_err(py, err, "extract native handler for flush");
                false
            }
        }
    }
}

/// Flush one wrapped built-in handler with the interpreter released.
///
/// The shard worker attaches the interpreter only to reach the Rust handler
/// behind the Python object. Holding it through the handler's queue wait would
/// block every other Python thread for the whole flush budget, so the wait
/// runs detached. The borrow is safe to carry across the release: the caller
/// owns the Python object for the duration of the call, so the handler cannot
/// be deallocated, and a concurrent `PyRefMut` would fail its borrow check
/// rather than alias this shared reference.
fn flush_detached<T>(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    context: &FlushContext,
) -> PyResult<bool>
where
    T: FemtoHandlerTrait + PyClass + Sync,
{
    let handler = obj.extract::<PyRef<'_, T>>()?;
    let handler_ref: &T = &handler;
    Ok(py.detach(|| FemtoHandlerTrait::flush_until(handler_ref, context)))
}
