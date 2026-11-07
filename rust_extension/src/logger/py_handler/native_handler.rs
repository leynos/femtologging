//! Native dispatch for built-in handlers registered through Python.

use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;
use pyo3::{Bound, PyAny};

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::handlers::{
    rotating::PyRotatingFileHandler, timed_rotating::PyTimedRotatingFileHandler,
};
use crate::log_record::FemtoLogRecord;
use crate::{FemtoFileHandler, FemtoHTTPHandler, FemtoSocketHandler, FemtoStreamHandler};

use super::map_py_err;

/// Flush a built-in handler extracted from its Python wrapper.
///
/// The wrapper is the only handle the logger holds, so the native handler is
/// reached by extracting it again. A failed extraction is reported as a failed
/// flush rather than silently succeeding: the caller asked for buffered records
/// to reach their destination, and a wrapper that cannot be unwrapped cannot
/// promise that.
fn flush_native<T>(py: Python<'_>, obj: &Bound<'_, PyAny>, label: &str) -> bool
where
    T: FemtoHandlerTrait + pyo3::PyClass,
{
    match obj.extract::<PyRef<'_, T>>() {
        Ok(handler) => FemtoHandlerTrait::flush(&*handler),
        Err(err) => {
            map_py_err(py, err.into(), label);
            false
        }
    }
}

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

    /// Flush the matching built-in handler through its native implementation.
    ///
    /// Going through the trait rather than a Python-visible `flush` method
    /// means a handler wrapped for Python flushes even when it exposes no such
    /// method — `FemtoFileHandler` buffers records without exposing `flush`,
    /// so relying on the Python attribute would report success while records
    /// stayed in its buffer.
    pub(super) fn flush(self, py: Python<'_>, obj: &Bound<'_, PyAny>) -> bool {
        match self {
            Self::Stream => {
                flush_native::<FemtoStreamHandler>(py, obj, "extract FemtoStreamHandler")
            }
            Self::File => flush_native::<FemtoFileHandler>(py, obj, "extract FemtoFileHandler"),
            Self::RotatingFile => {
                flush_native::<PyRotatingFileHandler>(py, obj, "extract FemtoRotatingFileHandler")
            }
            Self::TimedRotatingFile => flush_native::<PyTimedRotatingFileHandler>(
                py,
                obj,
                "extract FemtoTimedRotatingFileHandler",
            ),
            Self::Socket => {
                flush_native::<FemtoSocketHandler>(py, obj, "extract FemtoSocketHandler")
            }
            Self::Http => flush_native::<FemtoHTTPHandler>(py, obj, "extract FemtoHTTPHandler"),
        }
    }
}
