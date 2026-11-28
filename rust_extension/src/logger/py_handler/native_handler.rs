//! Native dispatch for built-in handlers registered through Python.
//!
//! `NativeHandlerKind::from_object` recognises each wrapped built-in — the
//! stream, file, rotating, timed-rotating, socket and HTTP handlers — so
//! `handle` can extract the native recipient with `PyRef` and deliver the
//! complete `FemtoLogRecord` rather than a lossy Python view. `flush` and
//! `flush_until` reach the native flush through `flush_native` and
//! `flush_detached`, releasing the interpreter so a handler's queue wait
//! does not block other Python threads.

use log::warn;
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

/// Flush a built-in handler extracted from its Python wrapper.
///
/// The wrapper is the only handle the logger holds, so the native handler is
/// reached by extracting it again. A failed extraction is reported as a failed
/// flush rather than silently succeeding: the caller asked for buffered records
/// to reach their destination, and a wrapper that cannot be unwrapped cannot
/// promise that.
///
/// The handler kind is carried into both diagnostics: a failed flush names the
/// built-in handler that failed, so the `False` from `flush_handlers()` points
/// at one handler rather than only at the call.
fn flush_native<T>(py: Python<'_>, obj: &Bound<'_, PyAny>, kind: NativeHandlerKind) -> bool
where
    T: FemtoHandlerTrait + pyo3::PyClass,
{
    match obj.extract::<PyRef<'_, T>>() {
        // A native flush waits for the handler's worker, and that worker may
        // be running a Python formatter that needs the GIL. Holding the GIL
        // here would deadlock until the handler's flush deadline expired, so
        // the wait runs detached. Borrowing as `&T` keeps the closure free of
        // the `PyRef` guard while the handler stays alive in `obj`.
        Ok(handler) => {
            let handler_ref: &T = &handler;
            let flushed =
                crate::py_detach::detach_for_wait(py, || FemtoHandlerTrait::flush(handler_ref));
            if !flushed {
                warn_flush_failed(kind);
            }
            flushed
        }
        Err(err) => {
            map_py_err(py, &err.into(), kind.extract_label());
            false
        }
    }
}

/// Record a built-in handler that did not confirm its flush.
///
/// `flush_handlers()` reports these failures, so an operator needs a bounded
/// record of the handler failing to flush, naming which built-in did, rather
/// than only a `False` return. Both flush paths use this so the warning is the
/// same whichever entry point reached the handler.
fn warn_flush_failed(kind: NativeHandlerKind) {
    warn!(
        "PyHandler: flush failed: handler_kind={}, category=flush_failed",
        kind.label()
    );
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

    /// A bounded label for the built-in handler, for structured logs and
    /// spans.
    ///
    /// This names the handler type rather than the `native`/`custom` split:
    /// when `flush_handlers()` reports a failure, an operator needs to know
    /// which built-in handler failed, not only that it was a native one.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::File => "file",
            Self::RotatingFile => "rotating-file",
            Self::TimedRotatingFile => "timed-rotating-file",
            Self::Socket => "socket",
            Self::Http => "http",
        }
    }

    /// The operation named when extracting this handler from its wrapper
    /// fails.
    ///
    /// This is the existing diagnostic text, kept per variant so the message
    /// names the type the caller expected to find in the wrapper.
    const fn extract_label(self) -> &'static str {
        match self {
            Self::Stream => "extract FemtoStreamHandler",
            Self::File => "extract FemtoFileHandler",
            Self::RotatingFile => "extract FemtoRotatingFileHandler",
            Self::TimedRotatingFile => "extract FemtoTimedRotatingFileHandler",
            Self::Socket => "extract FemtoSocketHandler",
            Self::Http => "extract FemtoHTTPHandler",
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
                .map_err(|err| map_py_err(py, &err.into(), "extract FemtoStreamHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
            Self::File => obj
                .extract::<PyRef<'_, FemtoFileHandler>>()
                .map_err(|err| map_py_err(py, &err.into(), "extract FemtoFileHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
            Self::RotatingFile => obj
                .extract::<PyRef<'_, PyRotatingFileHandler>>()
                .map_err(|err| map_py_err(py, &err.into(), "extract FemtoRotatingFileHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
            Self::TimedRotatingFile => obj
                .extract::<PyRef<'_, PyTimedRotatingFileHandler>>()
                .map_err(|err| map_py_err(py, &err.into(), "extract FemtoTimedRotatingFileHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
            Self::Socket => obj
                .extract::<PyRef<'_, FemtoSocketHandler>>()
                .map_err(|err| map_py_err(py, &err.into(), "extract FemtoSocketHandler"))
                .and_then(|handler| FemtoHandlerTrait::handle(&*handler, record)),
            Self::Http => obj
                .extract::<PyRef<'_, FemtoHTTPHandler>>()
                .map_err(|err| map_py_err(py, &err.into(), "extract FemtoHTTPHandler"))
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
            Self::Stream => flush_native::<FemtoStreamHandler>(py, obj, self),
            Self::File => flush_native::<FemtoFileHandler>(py, obj, self),
            Self::RotatingFile => flush_native::<PyRotatingFileHandler>(py, obj, self),
            Self::TimedRotatingFile => flush_native::<PyTimedRotatingFileHandler>(py, obj, self),
            Self::Socket => flush_native::<FemtoSocketHandler>(py, obj, self),
            Self::Http => flush_native::<FemtoHTTPHandler>(py, obj, self),
        }
    }

    /// Flush the matching built-in handler under the supplied deadline.
    ///
    /// A Python registration must not report a confirmed barrier while the
    /// wrapped handler still buffers records, so the handler's own
    /// deadline-aware flush runs here. Failing to extract the wrapped handler
    /// returns `false`: the barrier cannot confirm a flush it did not perform.
    /// A handler that declines the flush is recorded with the same handler-kind
    /// warning the legacy `flush` path emits, so the barrier's `False` still
    /// points at one built-in handler.
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
            Ok(true) => true,
            Ok(false) => {
                warn_flush_failed(self);
                false
            }
            Err(err) => {
                let _ = map_py_err(py, &err, "extract native handler for flush");
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
    Ok(crate::py_detach::detach_for_wait(py, || {
        FemtoHandlerTrait::flush_until(handler_ref, context)
    }))
}
