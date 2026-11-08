//! Flush support for handlers registered through Python.
//!
//! `flush_handlers` must reach the handler behind the Python wrapper, not just
//! the dispatch shard. A built-in handler is flushed through its native
//! implementation so the wait releases the GIL; a user-defined handler is
//! asked to flush through its own method.

use log::warn;
use pyo3::exceptions::PyAttributeError;
use pyo3::prelude::*;
use pyo3::types::PyAnyMethods;

use super::PyHandler;

impl PyHandler {
    /// Flush the wrapped handler, attaching the interpreter only to reach it.
    ///
    /// A built-in handler is flushed through its native implementation, which
    /// releases the GIL for the wait. A user-defined handler is asked to flush
    /// through its own method. The interpreter is attached only to reach the
    /// handler; the native wait itself runs detached.
    ///
    /// An interpreter that cannot be attached — because it is shutting down —
    /// leaves the handler unflushed, and that is reported as a failed flush:
    /// the wrapper has no evidence the handler's buffered records reached their
    /// destination, so claiming success would let `flush_handlers` return
    /// `True` over records that were never written.
    pub(super) fn flush_registered_handler(&self) -> bool {
        let flushed = self.report_flush(|| {
            Python::try_attach(|py| match self.native_handler {
                Some(native_handler) => native_handler.flush(py, self.obj.bind(py)),
                None => self.call_python_flush(py),
            })
        });
        #[cfg(feature = "tracing-compat")]
        tracing::trace!(
            target: "femtologging::internal",
            operation = "flush_registered_handler",
            handler_kind = self.handler_kind(),
            flush_outcome = if flushed { "succeeded" } else { "failed" },
        );
        flushed
    }

    /// Report the result of an attach attempt, treating an unavailable
    /// interpreter as a failed flush.
    ///
    /// The attempt is injected rather than performed here so the failure
    /// branch is testable: `Python::try_attach` returns `None` only while the
    /// interpreter is finalizing, which no test can arrange.
    pub(super) fn report_flush<F>(&self, attempt: F) -> bool
    where
        F: FnOnce() -> Option<bool>,
    {
        match attempt() {
            Some(flushed) => flushed,
            None => {
                warn!(
                    "PyHandler: flush failed: handler_kind={}, category=interpreter_unavailable",
                    self.handler_kind()
                );
                false
            }
        }
    }

    /// A bounded label for the wrapped handler kind, for structured logs.
    fn handler_kind(&self) -> &'static str {
        if self.native_handler.is_some() {
            "native"
        } else {
            "custom"
        }
    }

    /// Flush a handler that is not one of the built-in native handlers.
    ///
    /// Python handlers only have to implement `handle`, so a missing `flush`
    /// attribute means there is nothing to flush and counts as success, as does
    /// an attribute that is present but not callable. Any other failure to read
    /// the attribute — a property that raises, for instance — is reported as a
    /// failed flush, because the wrapper could not establish that there was
    /// nothing to flush. A `flush` that raises is reported as a failure; one
    /// that returns `None` or any other non-boolean value, an integer such as
    /// `0` included, counts as success. Only a Python `bool` is read for its
    /// truth, because that is all `extract::<bool>` accepts.
    ///
    /// A Python exception is printed for the user, as one raised from `handle`
    /// is. The structured log carries only bounded labels — the handler kind
    /// and whether the lookup or the call failed — because the exception text
    /// may contain record payloads or other sensitive data.
    fn call_python_flush(&self, py: Python<'_>) -> bool {
        let method = match self.obj.bind(py).getattr("flush") {
            Ok(method) => method,
            Err(err) if err.is_instance_of::<PyAttributeError>(py) => return true,
            Err(err) => {
                report_python_flush_failure(py, err, self.handler_kind(), "flush_lookup");
                return false;
            }
        };
        if !method.is_callable() {
            return true;
        }
        match method.call0() {
            Ok(result) => result.extract::<bool>().unwrap_or(true),
            Err(err) => {
                report_python_flush_failure(py, err, self.handler_kind(), "flush_call");
                false
            }
        }
    }
}

/// Print a Python flush exception and log a bounded failure category.
///
/// Printing keeps the traceback available to the user, matching the `handle`
/// path. Only the bounded category and handler kind reach the structured log.
fn report_python_flush_failure(
    py: Python<'_>,
    err: PyErr,
    handler_kind: &'static str,
    category: &'static str,
) {
    err.print(py);
    warn!("PyHandler: flush failed: handler_kind={handler_kind}, category={category}");
}
