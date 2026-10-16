//! Python methods exposed by the `FemtoLogger` PyO3 class.

use std::sync::{Arc, atomic::Ordering};

use pyo3::prelude::*;
use pyo3::{Py, PyAny};

use crate::{handler::FemtoHandlerTrait, level::FemtoLevel};

use super::{FemtoLogger, HandlerAttachment, PyHandler, validate_handler};

#[allow(
    clippy::too_many_arguments,
    reason = "PyO3 generates Python-exposed wrappers and signature shims with many parameters"
)]
#[pymethods]
impl FemtoLogger {
    /// Create a new logger with the given name.
    #[new]
    #[pyo3(text_signature = "(name)")]
    pub fn new(name: String) -> Self {
        Self::with_parent(name, None)
    }

    /// Update the logger's minimum level.
    ///
    /// `level` accepts "TRACE", "DEBUG", "INFO", "WARN", "ERROR", or
    /// "CRITICAL". The update is thread‑safe because the level is stored in an
    /// `AtomicU8`.
    #[pyo3(text_signature = "(self, level)")]
    pub fn set_level(&self, level: FemtoLevel) {
        self.level.store(u8::from(level), Ordering::Relaxed);
    }

    /// Return the logger's current minimum level as a string.
    ///
    /// This method is thread-safe; the level is stored in an `AtomicU8` and
    /// read with `Ordering::Relaxed`.
    #[getter]
    pub fn level(&self) -> String {
        self.load_level().to_string()
    }

    /// Return whether this logger propagates records to its parent (affecting parent-propagation behaviour).
    #[getter]
    pub fn propagate(&self) -> bool {
        self.propagate.load(Ordering::SeqCst)
    }

    /// Return whether configuration has disabled this logger.
    ///
    /// A disabled logger rejects records in its own producer path, so it emits
    /// nothing locally and nothing reaches its ancestors. The flag is set by
    /// `ConfigBuilder::build_and_init` when `disable_existing_loggers` omits
    /// this logger, and cleared when a later configuration names it
    /// explicitly.
    #[getter]
    pub fn disabled(&self) -> bool {
        self.is_disabled()
    }

    /// Set whether this logger propagates records to its parent, controlling parent-propagation behaviour.
    #[pyo3(text_signature = "(self, flag)")]
    pub fn set_propagate(&self, flag: bool) {
        self.propagate.store(flag, Ordering::SeqCst);
    }

    /// Attach a handler implemented in Python or Rust.
    #[pyo3(name = "add_handler", text_signature = "(self, handler)")]
    pub fn py_add_handler(&self, handler: Py<PyAny>) -> PyResult<()> {
        Python::attach(|py| {
            let obj = handler.bind(py);
            validate_handler(obj)?;
            let py_handler = PyHandler::new(py, handler);
            self.add_handler(Arc::new(py_handler) as Arc<dyn FemtoHandlerTrait>)?;
            Ok(())
        })
    }

    /// Remove a handler that was previously attached via `add_handler`.
    #[pyo3(name = "remove_handler", text_signature = "(self, handler)")]
    pub fn py_remove_handler(&self, handler: Py<PyAny>) -> bool {
        Python::attach(|py| {
            let mut handlers = self.handlers.write();
            let matches_handler = |attachment: &HandlerAttachment| {
                attachment
                    .handler
                    .as_any()
                    .downcast_ref::<PyHandler>()
                    .is_some_and(|py_h| py_h.obj.bind(py).is(handler.bind(py)))
            };
            if let Some(pos) = handlers.iter().position(matches_handler) {
                handlers.remove(pos);
                true
            } else {
                false
            }
        })
    }

    /// Remove all attached handlers.
    #[pyo3(name = "clear_handlers", text_signature = "(self)")]
    pub fn py_clear_handlers(&self) {
        self.clear_handlers();
    }

    /// Remove all attached filters.
    #[pyo3(name = "clear_filters", text_signature = "(self)")]
    pub fn py_clear_filters(&self) {
        self.clear_filters();
    }

    /// Return the number of records dropped due to a full or shutting-down queue.
    ///
    /// Useful for tests and monitoring dashboards.
    #[pyo3(text_signature = "(self)")]
    pub fn get_dropped(&self) -> u64 {
        self.dropped_records.load(Ordering::Relaxed)
    }

    /// Return how many records were dropped because producer context capture failed.
    #[cfg(feature = "python")]
    #[pyo3(text_signature = "(self)")]
    pub fn get_context_capture_failures(&self) -> u64 {
        self.context_capture_failures.load(Ordering::Relaxed)
    }

    /// Flush all handlers attached to this logger, releasing the GIL while waiting.
    ///
    /// First waits up to 2 seconds for the internal worker thread to drain
    /// its queue, then calls ``flush()`` on every attached handler (each
    /// handler applies its own timeout).
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` when the worker drains in time and every handler flush
    ///     succeeds.
    ///     ``False`` when the worker queue cannot be drained (channel
    ///     closed or timeout exceeded) or any handler flush returns
    ///     ``False``.
    ///
    /// Examples
    /// --------
    /// >>> logger.flush_handlers()
    /// True
    #[pyo3(name = "flush_handlers", text_signature = "(self)")]
    fn py_flush_handlers(&self, py: Python<'_>) -> bool {
        py.detach(|| self.flush_handlers())
    }

    /// Capture handler addresses for tests that verify attachment replacement.
    fn handler_ptrs_for_test(&self) -> Vec<usize> {
        self.handlers
            .read()
            .iter()
            .map(|attachment| Arc::as_ptr(&attachment.handler) as *const () as usize)
            .collect()
    }

    fn filter_ptrs_for_test(&self) -> Vec<usize> {
        self.filters
            .read()
            .iter()
            .map(|f| Arc::as_ptr(f) as *const () as usize)
            .collect()
    }
}

