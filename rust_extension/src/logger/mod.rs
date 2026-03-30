//! Core logger implementation for the FemtoLogger system.
//!
//! This module provides the [`FemtoLogger`] struct which handles log message
//! filtering, formatting, and asynchronous output via a background thread.
#![allow(
    clippy::too_many_arguments,
    reason = "PyO3 macro-generated wrappers expand Python-call signatures"
)]

mod convenience_methods;
mod producer;
mod py_handler;
mod python_helpers;
#[cfg(feature = "python")]
mod runtime_mutation;
mod worker;

use pyo3::prelude::*;
use pyo3::{Py, PyAny};
use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::filters::FemtoFilter;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_context;
use crate::rate_limited_warner::RateLimitedWarner;
#[cfg(feature = "python")]
use crate::traceback_capture;

use crate::{
    formatter::SharedFormatter,
    level::FemtoLevel,
    log_record::{FemtoLogRecord, RecordMetadata},
};
use crossbeam_channel::Sender;
// parking_lot avoids poisoning and matches crate-wide locking strategy
use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::thread::JoinHandle;

pub use py_handler::{PyHandler, validate_handler};
#[cfg(feature = "python")]
pub use python_helpers::should_capture_exc_info;

const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
const LOGGER_FLUSH_TIMEOUT_MS: u64 = 2_000;

/// Handler used internally to acknowledge logger flush operations.
struct FlushAckHandler {
    ack: Sender<()>,
}

impl FlushAckHandler {
    fn new(ack: Sender<()>) -> Self {
        Self { ack }
    }
}

impl FemtoHandlerTrait for FlushAckHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        let _ = self.ack.send(());
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Record queued for processing by the worker thread.
pub struct QueuedRecord {
    pub record: FemtoLogRecord,
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
    /// # Returns
    ///
    /// The formatted log message if the record passes level and filter checks,
    /// otherwise `None`.
    #[pyo3(
        name = "log",
        signature = (level, message, /, *, exc_info=None, stack_info=false),
        text_signature = "(self, level, message, /, *, exc_info=None, stack_info=False)"
    )]
    #[cfg_attr(
        not(feature = "python"),
        expect(
            unused_variables,
            reason = "py parameter is only used when python feature is enabled"
        )
    )]
    #[cfg_attr(
        not(feature = "python"),
        expect(
            unused_mut,
            reason = "record is only mutated when python feature is enabled"
        )
    )]
    pub fn py_log(
        &self,
        py: Python<'_>,
        level: FemtoLevel,
        message: &str,
        exc_info: Option<&Bound<'_, PyAny>>,
        stack_info: Option<bool>,
    ) -> PyResult<Option<String>> {
        if !self.is_enabled_for(level) {
            return Ok(None);
        }
        let explicit_key_values = BTreeMap::new();
        let merged_key_values = match log_context::merge_context_values(&explicit_key_values) {
            Ok(key_values) => key_values,
            Err(err) => {
                eprintln!("FemtoLogger: dropping record due to invalid context payload: {err}");
                return Ok(None);
            }
        };
        let mut record = FemtoLogRecord::with_metadata(
            &self.name,
            level,
            message,
            RecordMetadata {
                key_values: merged_key_values,
                ..Default::default()
            },
        );

        // Capture exception payload if exc_info is provided and truthy
        #[cfg(feature = "python")]
        if let Some(exc) = exc_info
            && should_capture_exc_info(exc)?
            && let Some(payload) = traceback_capture::capture_exception(py, exc)?
        {
            record.set_exception_payload(payload);
        }

        // Capture stack payload if stack_info=True
        #[cfg(feature = "python")]
        if stack_info.unwrap_or(false) {
            record.set_stack_payload(traceback_capture::capture_stack(py)?);
        }

        Ok(self.log_record(record))
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
    pub fn py_remove_handler(&self, handler: Py<PyAny>) -> bool {
        Python::attach(|py| {
            let mut handlers = self.handlers.write();
            let matches_handler = |h: &Arc<dyn FemtoHandlerTrait>| {
                h.as_any()
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
            .map(|h| Arc::as_ptr(h) as *const () as usize)
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
        if let Some(pos) = handlers.iter().position(|h| Arc::ptr_eq(h, handler)) {
            handlers.remove(pos);
            true
        } else {
            false
        }
    }

    /// Remove all handlers from this logger.
    ///
    /// Note: This affects only records enqueued after the call. Any records
    /// already queued retain their captured handler set and will still be
    /// dispatched to those handlers.
    pub fn clear_handlers(&self) {
        self.handlers.write().clear();
    }

    pub fn remove_filter(&self, filter: &Arc<dyn FemtoFilter>) -> bool {
        let mut filters = self.filters.write();
        if let Some(pos) = filters.iter().position(|f| Arc::ptr_eq(f, filter)) {
            filters.remove(pos);
            true
        } else {
            false
        }
    }

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
        self.tx.as_ref().cloned()
    }
}

impl Drop for FemtoLogger {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.tx.take();
        // Drop the lock before joining the worker thread.
        let handle = { self.handle.lock().take() };
        if let Some(handle) = handle {
            Python::attach(|py| py.detach(move || worker::log_join_result(handle)));
        }
    }
}

#[cfg(test)]
#[path = "logger_tests.rs"]
mod logger_tests;
