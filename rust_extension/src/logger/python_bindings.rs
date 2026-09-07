//! `PyO3` methods for [`super::FemtoLogger`].
//!
//! This module isolates generated Python method wrappers from the logger core.

use std::sync::{Arc, atomic::Ordering};

use pyo3::{
    prelude::*,
    types::{PyDict, PyTuple},
};

use super::{FemtoLogger, PyHandler, log_python_request, parse_log_call, validate_handler};
use crate::{handler::FemtoHandlerTrait, level::FemtoLevel};

#[pymethods]
impl FemtoLogger {
    /// Create a new logger with the given name.
    #[new]
    #[pyo3(text_signature = "(name)")]
    #[must_use]
    pub fn new(name: String) -> Self { Self::with_parent(name, None) }

    /// Format a message at the provided level and return it.
    ///
    /// This method builds a log record, optionally capturing exception and
    /// stack trace information if `exc_info` or `stack_info` are provided.
    ///
    /// # Parameters
    ///
    /// - `level`: The log level (e.g., "INFO", "ERROR").
    /// - `message`: The log message.
    /// - `exc_info`: Optional exception information. Accepts:
    ///   - `True`: Capture the current exception via `sys.exc_info()`.
    ///   - An exception instance: Capture that exception's traceback.
    ///   - A 3-tuple `(type, value, traceback)`: Use directly.
    /// - `stack_info`: If `True`, capture the current call stack.
    ///
    /// # Errors
    ///
    /// Returns a Python error when the call does not match the documented
    /// signature, its level is invalid, or exception capture fails.
    ///
    /// # Returns
    ///
    /// The formatted log message if the record passes level and filter checks,
    /// otherwise `None`.
    #[pyo3(
        name = "log",
        signature = (*args, **kwargs),
        text_signature = "(self, level, message, /, *, exc_info=None, stack_info=False)"
    )]
    pub fn py_log<'py>(
        &self,
        py: Python<'py>,
        args: &Bound<'py, PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Option<String>> {
        log_python_request(self, py, &parse_log_call(args, kwargs)?)
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
    pub fn level(&self) -> String { self.load_level().to_string() }

    /// Return whether this logger propagates records to its parent (affecting parent-propagation
    /// behaviour).
    #[getter]
    pub fn propagate(&self) -> bool { self.propagate.load(Ordering::SeqCst) }

    /// Set whether this logger propagates records to its parent, controlling parent-propagation
    /// behaviour.
    #[pyo3(text_signature = "(self, flag)")]
    pub fn set_propagate(&self, flag: bool) { self.propagate.store(flag, Ordering::SeqCst); }

    /// Attach a handler implemented in Python or Rust.
    ///
    /// # Errors
    ///
    /// Returns a Python error when `handler` lacks a callable `handle` method.
    #[pyo3(name = "add_handler", text_signature = "(self, handler)")]
    pub fn py_add_handler(&self, handler: Py<PyAny>) -> PyResult<()> {
        Python::attach(|py| {
            let obj = handler.bind(py);
            validate_handler(obj)?;
            let py_handler = PyHandler::new(py, handler);
            self.add_handler(Arc::new(py_handler) as Arc<dyn FemtoHandlerTrait>);
            Ok(())
        })
    }

    /// Remove a handler that was previously attached via `add_handler`.
    #[pyo3(name = "remove_handler", text_signature = "(self, handler)")]
    pub fn py_remove_handler(&self, handler: &Bound<'_, PyAny>) -> bool {
        Python::attach(|py| {
            let mut handlers = self.handlers.write();
            let matches_handler = |h: &Arc<dyn FemtoHandlerTrait>| {
                h.as_any()
                    .downcast_ref::<PyHandler>()
                    .is_some_and(|py_h| py_h.obj.bind(py).is(handler))
            };
            handlers
                .iter()
                .position(matches_handler)
                .is_some_and(|position| {
                    handlers.remove(position);
                    true
                })
        })
    }

    /// Remove all attached handlers.
    #[pyo3(name = "clear_handlers", text_signature = "(self)")]
    pub fn py_clear_handlers(&self) { self.clear_handlers(); }

    /// Remove all attached filters.
    #[pyo3(name = "clear_filters", text_signature = "(self)")]
    pub fn py_clear_filters(&self) { self.clear_filters(); }

    /// Return the number of records dropped due to a full queue.
    ///
    /// Useful for tests and monitoring dashboards.
    #[pyo3(text_signature = "(self)")]
    pub fn get_dropped(&self) -> u64 { self.dropped_records.load(Ordering::Relaxed) }

    /// Flush all handlers attached to this logger.
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
    #[pyo3(text_signature = "(self)")]
    pub fn flush_handlers(&self) -> bool { self.flush_handlers_blocking() }

    pub(super) fn handler_ptrs_for_test(&self) -> Vec<usize> {
        self.handlers
            .read()
            .iter()
            .map(|handler| Arc::as_ptr(handler).cast::<()>() as usize)
            .collect()
    }
}
