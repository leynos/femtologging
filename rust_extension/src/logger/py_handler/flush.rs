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
        // The span covers the queue-crossing wait itself, so the interval an
        // operator sees is the one this flush spent waiting on the handler,
        // not just the instant its outcome was logged.
        #[cfg(feature = "tracing-compat")]
        let span = tracing::info_span!(
            target: "femtologging::internal",
            "flush_registered_handler",
            handler_kind = self.handler_kind(),
            flush_outcome = tracing::field::Empty,
        );
        #[cfg(feature = "tracing-compat")]
        let _entered = span.enter();

        let flushed = self.report_flush(|| {
            Python::try_attach(|py| match self.native_handler {
                Some(native_handler) => native_handler.flush(py, self.obj.bind(py)),
                None => self.call_python_flush(py),
            })
        });
        #[cfg(feature = "tracing-compat")]
        {
            span.record(
                "flush_outcome",
                if flushed { "succeeded" } else { "failed" },
            );
            tracing::trace!(
                target: "femtologging::internal",
                operation = "flush_registered_handler",
                handler_kind = self.handler_kind(),
                flush_outcome = if flushed { "succeeded" } else { "failed" },
            );
        }
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
    ///
    /// A built-in handler reports its own type, so a failed
    /// `flush_handlers()` names the handler an operator has to look at
    /// rather than only the `native`/`custom` split.
    fn handler_kind(&self) -> &'static str {
        match self.native_handler {
            Some(kind) => kind.label(),
            None => "custom",
        }
    }
}
