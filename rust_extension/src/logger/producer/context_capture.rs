//! Python context capture for records that are about to be queued.
//!
//! A record carries the task-local context that was present when it was logged.
//! Only a queued Python-backed handler can observe that context, so the
//! snapshot is taken against the exact handler set being queued rather than
//! taken speculatively. A failed capture is counted separately from
//! queue-capacity drops so the two pressures stay distinguishable.

use log::warn;

use super::super::FemtoLogger;

impl FemtoLogger {
    /// Capture Python context for the exact handler snapshot being queued.
    pub(super) fn capture_context_for_dispatch(
        &self,
        handlers: &[std::sync::Arc<dyn crate::handler::FemtoHandlerTrait>],
        #[cfg(feature = "tracing-compat")] handler_kind: &'static str,
    ) -> pyo3::PyResult<Option<pyo3::Py<pyo3::PyAny>>> {
        #[cfg(feature = "tracing-compat")]
        let capture_started = std::time::Instant::now();
        let context = self.capture_python_context(handlers);
        #[cfg(feature = "tracing-compat")]
        {
            let capture_outcome = match &context {
                Ok(Some(_)) => "captured",
                Ok(None) => "not_required",
                Err(_) => "failed",
            };
            tracing::trace!(
                target: "femtologging::internal",
                operation = "producer_context_capture",
                handler_kind,
                capture_outcome,
                elapsed_us = capture_started.elapsed().as_micros(),
            );
        }
        context
    }

    /// Count a failed context snapshot separately from queue-capacity drops.
    pub(super) fn record_context_capture_failure(&self, error: pyo3::PyErr) {
        self.context_capture_failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.context_capture_warner.record_drop();
        self.context_capture_warner.warn_if_due(|count| {
            warn!("FemtoLogger: dropped {count} records after context capture failed");
            pyo3::Python::attach(|py| error.print(py));
        });
    }

    /// Capture the current Python context only when a queued handler needs it.
    pub(in crate::logger) fn capture_python_context(
        &self,
        handlers: &[std::sync::Arc<dyn crate::handler::FemtoHandlerTrait>],
    ) -> pyo3::PyResult<Option<pyo3::Py<pyo3::PyAny>>> {
        if !handlers.iter().any(|handler| handler.is_python_backed()) {
            return Ok(None);
        }
        self.context_snapshot_provider.capture().map(Some)
    }
}
