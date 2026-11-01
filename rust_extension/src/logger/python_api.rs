//! Python methods exposed by the `FemtoLogger` PyO3 class.

use std::sync::{Arc, atomic::Ordering};

use pyo3::prelude::*;
use pyo3::{Py, PyAny};

use crate::{handler::FemtoHandlerTrait, level::FemtoLevel};

use super::{FemtoLogger, HandlerAttachment, PyHandler, validate_handler};

#[pymethods]
impl FemtoLogger {
    /// Create a new logger with the given name.
    ///
    /// Names must satisfy the same rules as
    /// [`crate::manager::get_logger`]: at most
    /// [`MAX_LOGGER_NAME_BYTES`](crate::manager::MAX_LOGGER_NAME_BYTES) bytes,
    /// at most
    /// [`MAX_LOGGER_NAME_SEGMENTS`](crate::manager::MAX_LOGGER_NAME_SEGMENTS)
    /// dot-separated segments, no leading or trailing dot, no empty segment,
    /// and no line-breaking character: any control character (C0, DEL or C1),
    /// or the Unicode line and paragraph separators. A logger built this way
    /// is not registered in the global manager, so it does not count against
    /// the registry limit.
    ///
    /// Raises
    /// ------
    /// ValueError
    ///     If the name breaks the rules above.
    /// RuntimeError
    ///     If the process cannot start the shared dispatch pool.
    #[new]
    #[pyo3(text_signature = "(name)")]
    pub fn new(name: String) -> PyResult<Self> {
        crate::manager::validate_logger_name(&name)?;
        Ok(Self::with_parent(name, None)?)
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

    /// Return how many records were refused because the logger is disabled.
    ///
    /// The counter rises for records aimed at a logger that
    /// `disable_existing_loggers` has disabled, so a deployment can tell
    /// configuration-disabled traffic apart from queue pressure. A later
    /// configuration that names the logger re-enables it and stops the rise.
    #[pyo3(text_signature = "(self)")]
    pub fn get_disabled_drops(&self) -> u64 {
        self.disabled_dropped_records.load(Ordering::Relaxed)
    }

    /// Flush all handlers attached to this logger, releasing the GIL while waiting.
    ///
    /// First waits up to 2 seconds for the logger's dispatch shard to drain
    /// the records already queued for it, then calls ``flush()`` on every
    /// attached handler (each handler applies its own timeout).
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` when the dispatcher drains the queue in time and every
    ///     handler flush succeeds. ``False`` when the queued work cannot be
    ///     drained (the shard is gone or the wait times out) or any handler
    ///     flush returns ``False``.
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
    pub(crate) fn handler_ptrs_for_test(&self) -> Vec<usize> {
        self.handler_ptrs_for_test_native()
    }

    /// Capture filter addresses for tests that verify attachment replacement.
    pub(crate) fn filter_ptrs_for_test(&self) -> Vec<usize> {
        self.filter_ptrs_for_test_native()
    }
}
