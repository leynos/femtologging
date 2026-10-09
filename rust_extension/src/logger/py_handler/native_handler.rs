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

/// Built-in handlers that can retain the original Rust record across Python registration.
#[derive(Clone, Copy)]
pub(super) enum NativeHandlerKind {
    Stream,
    File,
    RotatingFile,
    TimedRotatingFile,
    Socket,
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
}
