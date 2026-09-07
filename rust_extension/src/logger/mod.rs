//! Core logger implementation for the [`FemtoLogger`] system.
//!
//! This module provides the [`FemtoLogger`] struct which handles log message
//! filtering, formatting, and asynchronous output via a background thread.
mod convenience_methods;
mod producer;
mod py_handler;
mod python_helpers;
#[cfg(feature = "python")]
mod runtime_mutation;
mod worker;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};
use pyo3::{Py, PyAny};
use std::sync::Arc;

use crate::filters::FemtoFilter;
use crate::handler::FemtoHandlerTrait;
use crate::rate_limited_warner::RateLimitedWarner;

use crate::{formatter::SharedFormatter, level::FemtoLevel, log_record::FemtoLogRecord};
use crossbeam_channel::Sender;
// parking_lot avoids poisoning and matches crate-wide locking strategy
use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::thread::JoinHandle;

pub use py_handler::{PyHandler, validate_handler};
// Re-exported for the parameterized tests in `logger_tests_python.rs`;
// production code reaches it through `capture_exception_payload`.
#[cfg(all(feature = "python", test))]
pub(crate) use python_helpers::should_capture_exc_info;
use python_helpers::{log_python_request, parse_log_call};

const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
const LOGGER_FLUSH_TIMEOUT_MS: u64 = 2_000;

/// Record queued for processing by the worker thread.
pub struct QueuedRecord {
    /// Record to process on the worker thread.
    pub record: FemtoLogRecord,
    /// Handlers captured when the record was queued.
    pub handlers: Vec<Arc<dyn FemtoHandlerTrait>>,
}

/// Basic logger used for early experimentation.
#[pyclass]
pub struct FemtoLogger {
    /// Identifier used to distinguish log messages from different loggers.
    name: String,
    /// Parent logger name for dotted hierarchy.
    #[pyo3(get)]
    parent: Option<String>,
    formatter: SharedFormatter,
    level: AtomicU8,
    propagate: AtomicBool,
    handlers: Arc<RwLock<Vec<Arc<dyn FemtoHandlerTrait>>>>,
    filters: Arc<RwLock<Vec<Arc<dyn FemtoFilter>>>>,
    dropped_records: AtomicU64,
    drop_warner: RateLimitedWarner,
    tx: Option<Sender<QueuedRecord>>,
    shutdown_tx: Option<Sender<()>>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

#[pymethods]
impl FemtoLogger {
    /// Create a new logger with the given name.
    #[new]
    #[pyo3(text_signature = "(name)")]
    #[must_use]
    pub fn new(name: String) -> Self {
        Self::with_parent(name, None)
    }

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
    pub fn level(&self) -> String {
        self.load_level().to_string()
    }

    /// Return whether this logger propagates records to its parent (affecting parent-propagation behaviour).
    #[getter]
    pub fn propagate(&self) -> bool {
        self.propagate.load(Ordering::SeqCst)
    }

    /// Set whether this logger propagates records to its parent, controlling parent-propagation behaviour.
    #[pyo3(text_signature = "(self, flag)")]
    pub fn set_propagate(&self, flag: bool) {
        self.propagate.store(flag, Ordering::SeqCst);
    }

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
    pub fn py_clear_handlers(&self) {
        self.clear_handlers();
    }

    /// Remove all attached filters.
    #[pyo3(name = "clear_filters", text_signature = "(self)")]
    pub fn py_clear_filters(&self) {
        self.clear_filters();
    }

    /// Return the number of records dropped due to a full queue.
    ///
    /// Useful for tests and monitoring dashboards.
    #[pyo3(text_signature = "(self)")]
    pub fn get_dropped(&self) -> u64 {
        self.dropped_records.load(Ordering::Relaxed)
    }

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
    pub fn flush_handlers(&self) -> bool {
        self.flush_handlers_blocking()
    }

    fn handler_ptrs_for_test(&self) -> Vec<usize> {
        self.handlers
            .read()
            .iter()
            .map(|handler| Arc::as_ptr(handler).cast::<()>() as usize)
            .collect()
    }
}

impl FemtoLogger {
    /// Attach a handler to this logger.
    pub fn add_handler(&self, handler: Arc<dyn FemtoHandlerTrait>) {
        self.handlers.write().push(handler);
    }

    /// Attach a filter to this logger.
    pub fn add_filter(&self, filter: Arc<dyn FemtoFilter>) {
        self.filters.write().push(filter);
    }

    /// Detach a handler previously added to this logger.
    pub fn remove_handler(&self, handler: &Arc<dyn FemtoHandlerTrait>) -> bool {
        let mut handlers = self.handlers.write();
        handlers
            .iter()
            .position(|current| Arc::ptr_eq(current, handler))
            .is_some_and(|position| {
                handlers.remove(position);
                true
            })
    }

    /// Remove all handlers from this logger.
    ///
    /// Note: This affects only records enqueued after the call. Any records
    /// already queued retain their captured handler set and will still be
    /// dispatched to those handlers.
    pub fn clear_handlers(&self) {
        self.handlers.write().clear();
    }

    /// Detach a filter previously added to this logger.
    pub fn remove_filter(&self, filter: &Arc<dyn FemtoFilter>) -> bool {
        let mut filters = self.filters.write();
        filters
            .iter()
            .position(|current| Arc::ptr_eq(current, filter))
            .is_some_and(|position| {
                filters.remove(position);
                true
            })
    }

    /// Remove all attached filters from this logger.
    pub fn clear_filters(&self) {
        self.filters.write().clear();
    }

    #[cfg(test)]
    pub fn handlers_for_test(&self) -> Vec<Arc<dyn FemtoHandlerTrait>> {
        self.handlers.read().clone()
    }

    /// Clone the internal sender for use in tests.
    ///
    /// # Warning
    /// Any cloned sender must be dropped before the logger can shut down.
    /// Holding a clone alive after dropping the logger will prevent the worker
    /// thread from exiting.
    #[cfg(feature = "test-util")]
    pub fn clone_sender_for_test(&self) -> Option<Sender<QueuedRecord>> {
        self.tx.clone()
    }
}

impl Drop for FemtoLogger {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take()
            && shutdown_tx.send(()).is_err()
        {
            // The worker has already exited, but is still joined below so
            // a panic can be observed and reported.
        }
        self.tx.take();
        // Drop the lock before joining the worker thread.
        let worker_handle = { self.handle.lock().take() };
        if let Some(handle_to_join) = worker_handle {
            Python::attach(|py| py.detach(move || worker::log_join_result(handle_to_join)));
        }
    }
}

#[cfg(test)]
#[path = "logger_tests.rs"]
mod logger_tests;
#[cfg(test)]
#[path = "logger_tests_helpers.rs"]
mod logger_tests_helpers;
#[cfg(test)]
#[path = "producer_tests.rs"]
mod producer_tests;
#[cfg(test)]
#[path = "worker_tests.rs"]
mod worker_tests;
