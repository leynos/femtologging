//! Flush support for handlers registered through Python.
//!
//! `flush_handlers` must reach the handler behind the Python wrapper, not just
//! the dispatch shard. A built-in handler is flushed through its native
//! implementation so the wait releases the GIL; a user-defined handler is
//! asked to flush through its own method. Those two shapes live in
//! [`super::native_handler`] and [`super::calls`]; this module owns the step
//! they share — attaching the interpreter — and what a flush reports when the
//! interpreter cannot be reached at all.

use log::warn;
use pyo3::prelude::*;

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
}