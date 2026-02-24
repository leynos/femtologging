//! Core logger implementation for the FemtoLogger system.
//!
//! This module provides the [`FemtoLogger`] struct which handles log message
//! filtering, formatting, and asynchronous output via a background thread.
#![allow(
    clippy::too_many_arguments,
    reason = "PyO3 macro-generated wrappers expand Python-call signatures"
)]

mod py_handler;
mod python_helpers;

use pyo3::prelude::*;
use pyo3::{Py, PyAny};
use std::any::Any;
use std::sync::Arc;

use crate::filters::FemtoFilter;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::manager;
use crate::rate_limited_warner::RateLimitedWarner;
#[cfg(feature = "python")]
use crate::traceback_capture;

use crate::{
    formatter::{DefaultFormatter, SharedFormatter},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
};
use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded, select};
use log::warn;
// parking_lot avoids poisoning and matches crate-wide locking strategy
use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

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
        // Create base record
        let mut record = FemtoLogRecord::new(&self.name, level, message);

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
    /// Core logging logic shared between Python and Rust APIs.
    ///
    /// Checks level threshold and filters, formats the record, and dispatches
    /// to handlers. Returns `Some(formatted_message)` if the record was logged,
    /// or `None` if it was filtered out.
    fn log_record(&self, record: FemtoLogRecord) -> Option<String> {
        let threshold = self.level.load(Ordering::Relaxed);
        if u8::from(record.level()) < threshold {
            return None;
        }

        if !self.passes_all_filters(&record) {
            return None;
        }
        let msg = self.formatter.format(&record);
        self.dispatch_to_handlers(record);
        Some(msg)
    }

    /// Log a message at the given level (Rust-only API).
    ///
    /// This method is a simplified version of the PyO3-exposed `log` method
    /// for pure Rust callers. It does not support `exc_info` or `stack_info`.
    pub fn log(&self, level: FemtoLevel, message: &str) -> Option<String> {
        let record = FemtoLogRecord::new(&self.name, level, message);
        self.log_record(record)
    }

    /// Return whether `level` is enabled for this logger.
    #[cfg(feature = "log-compat")]
    pub(crate) fn is_enabled_for(&self, level: FemtoLevel) -> bool {
        u8::from(level) >= self.level.load(Ordering::Relaxed)
    }

    /// Dispatch an already-constructed record through this logger.
    ///
    /// The record is filtered against the logger's level and filters before
    /// being enqueued for handler processing.
    #[cfg(feature = "log-compat")]
    pub(crate) fn dispatch_record(&self, record: FemtoLogRecord) {
        if !self.is_enabled_for(record.level()) {
            return;
        }
        if !self.passes_all_filters(&record) {
            return;
        }
        self.dispatch_to_handlers(record);
    }

    /// Return the logger's current minimum level.
    ///
    /// This method is thread-safe; the level is stored in an `AtomicU8` and
    /// read with `Ordering::Relaxed`.
    pub fn get_level(&self) -> FemtoLevel {
        self.load_level()
    }

    /// Load the current level from the atomic storage.
    ///
    /// The level is guaranteed to be a valid `FemtoLevel` discriminant because
    /// the only way to set it is through `set_level`, which accepts a typed
    /// `FemtoLevel` value.
    fn load_level(&self) -> FemtoLevel {
        match self.level.load(Ordering::Relaxed) {
            0 => FemtoLevel::Trace,
            1 => FemtoLevel::Debug,
            2 => FemtoLevel::Info,
            3 => FemtoLevel::Warn,
            4 => FemtoLevel::Error,
            // Default case covers Critical (5) and any unexpected value.
            // In practice, only 0–5 are stored because `set_level` takes a
            // typed `FemtoLevel`.
            _ => FemtoLevel::Critical,
        }
    }

    /// Return `true` if every configured filter approves the record.
    ///
    /// Iterates over each filter and returns `false` on the first rejection.
    /// If no filters are configured, the record passes.
    fn passes_all_filters(&self, record: &FemtoLogRecord) -> bool {
        for f in self.filters.read().iter() {
            if !f.should_log(record) {
                return false;
            }
        }
        true
    }

    fn should_propagate_to_parent(&self) -> bool {
        self.propagate.load(Ordering::SeqCst) && self.parent.is_some()
    }

    fn handle_parent_propagation(&self, record: FemtoLogRecord) {
        let Some(parent_name) = &self.parent else {
            return;
        };
        Python::attach(|py| {
            if let Ok(parent) = manager::get_logger(py, parent_name) {
                parent.borrow(py).dispatch_to_handlers(record);
            }
        });
    }

    fn send_to_local_handlers(&self, record: FemtoLogRecord) {
        let Some(tx) = &self.tx else {
            return;
        };
        let handlers = self.handlers.read().clone();
        if tx.try_send(QueuedRecord { record, handlers }).is_ok() {
            return;
        }
        self.dropped_records.fetch_add(1, Ordering::Relaxed);
        self.drop_warner.record_drop();
        self.drop_warner.warn_if_due(|count| {
            warn!("FemtoLogger: dropped {count} records; queue full or shutting down");
        });
    }

    fn flush_handlers_blocking(&self) -> bool {
        self.wait_for_worker_idle() && self.flush_configured_handlers()
    }

    fn wait_for_worker_idle(&self) -> bool {
        let Some(tx) = &self.tx else {
            return true;
        };
        let (ack_tx, ack_rx) = bounded(1);
        let ack_handler: Arc<dyn FemtoHandlerTrait> = Arc::new(FlushAckHandler::new(ack_tx));
        let record = FemtoLogRecord::new("__femtologging__", FemtoLevel::Info, "__flush__");
        if tx
            .send(QueuedRecord {
                record,
                handlers: vec![ack_handler],
            })
            .is_err()
        {
            return false;
        }
        ack_rx
            .recv_timeout(Duration::from_millis(LOGGER_FLUSH_TIMEOUT_MS))
            .is_ok()
    }

    fn flush_configured_handlers(&self) -> bool {
        self.handlers.read().iter().all(|handler| handler.flush())
    }

    /// Dispatch a record to the logger's handlers via the background queue.
    ///
    /// The record is sent to the worker thread if a channel is configured.
    /// When the queue is full or the logger is shutting down, the record is
    /// dropped; a drop counter increments and a rate-limited warning is
    /// emitted.
    fn dispatch_to_handlers(&self, record: FemtoLogRecord) {
        let parent_record = self.should_propagate_to_parent().then(|| record.clone());
        self.send_to_local_handlers(record);
        if let Some(pr) = parent_record {
            self.handle_parent_propagation(pr);
        }
    }
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

    /// Create a logger with an explicit parent name.
    pub fn with_parent(name: String, parent: Option<String>) -> Self {
        let formatter = SharedFormatter::new(DefaultFormatter);
        let handlers: Arc<RwLock<Vec<Arc<dyn FemtoHandlerTrait>>>> =
            Arc::new(RwLock::new(Vec::new()));
        let filters: Arc<RwLock<Vec<Arc<dyn FemtoFilter>>>> = Arc::new(RwLock::new(Vec::new()));

        let (tx, rx) = bounded::<QueuedRecord>(DEFAULT_CHANNEL_CAPACITY);
        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);
        let handle = thread::spawn(move || {
            Self::worker_thread_loop(rx, shutdown_rx);
        });

        Self {
            name,
            parent,
            formatter,
            level: AtomicU8::new(u8::from(FemtoLevel::Info)),
            propagate: AtomicBool::new(true),
            handlers,
            filters,
            dropped_records: AtomicU64::new(0),
            drop_warner: RateLimitedWarner::default(),
            tx: Some(tx),
            shutdown_tx: Some(shutdown_tx),
            handle: Mutex::new(Some(handle)),
        }
    }

    /// Process a single `FemtoLogRecord` by dispatching it to all handlers.
    fn handle_log_record(job: QueuedRecord) {
        for h in job.handlers.iter() {
            if let Err(err) = h.handle(job.record.clone()) {
                warn!("FemtoLogger: handler reported an error: {err}");
            }
        }
    }

    /// Drain any remaining records once a shutdown signal is received.
    ///
    /// Consumes all messages still available on `rx` and dispatches them
    /// through the provided `handlers`. This ensures no log records are lost
    /// during shutdown.
    ///
    /// # Arguments
    ///
    /// * `rx` - Channel receiver holding pending log records.
    fn drain_remaining_records(rx: &Receiver<QueuedRecord>) {
        while let Ok(job) = rx.try_recv() {
            Self::handle_log_record(job);
        }
    }

    /// Finalize the worker thread by draining any remaining queued
    /// records.
    ///
    /// Acts as the shutdown entry point for the worker loop. The
    /// drain step ensures that records already enqueued at the moment
    /// shutdown was signalled are not silently lost.
    ///
    /// # Arguments
    ///
    /// * `rx` - Channel receiver holding pending log records.
    fn shutdown_and_drain(rx: &Receiver<QueuedRecord>) {
        Self::drain_remaining_records(rx);
    }

    /// Perform a non-blocking check for a pending shutdown signal.
    ///
    /// This is the Phase 1 check in the two-phase shutdown pattern
    /// used by [`worker_thread_loop`]. It uses `try_recv` rather
    /// than a blocking receive so the worker can detect a shutdown
    /// request that arrived while the previous `select!` iteration
    /// was busy processing a log record. Without this check, a
    /// continuously saturated record channel could delay shutdown
    /// recognition indefinitely.
    ///
    /// Returns `true` when the shutdown channel carries a message or
    /// has been disconnected.
    ///
    /// # Arguments
    ///
    /// * `shutdown_rx` - Channel receiver carrying the shutdown
    ///   signal.
    fn should_shutdown_now(shutdown_rx: &Receiver<()>) -> bool {
        matches!(
            shutdown_rx.try_recv(),
            Ok(()) | Err(TryRecvError::Disconnected)
        )
    }

    /// Main loop executed by the logger's worker thread.
    ///
    /// Uses a two-phase shutdown pattern to guarantee prompt shutdown
    /// even under sustained high-throughput logging:
    ///
    /// - **Phase 1** ([`should_shutdown_now`]): A non-blocking
    ///   `try_recv` on the shutdown channel, executed at the top of
    ///   every iteration *before* the blocking `select!`. This
    ///   provides a deterministic opportunity to observe a shutdown
    ///   signal that arrived while the previous iteration was
    ///   servicing a log record.
    ///
    /// - **Phase 2** (`select!`): A blocking wait on both the
    ///   shutdown and record channels. Although `crossbeam`'s
    ///   `select!` uses random selection when multiple channels are
    ///   ready, a continuously saturated record channel could still
    ///   cause the shutdown branch to lose repeated coin-flips,
    ///   delaying exit. Phase 1 eliminates this probabilistic delay
    ///   by guaranteeing that every loop iteration checks for
    ///   shutdown deterministically.
    ///
    /// When either phase detects a shutdown signal, all remaining
    /// queued records are drained before the thread exits so that no
    /// log messages are silently lost.
    ///
    /// # Arguments
    ///
    /// * `rx` - Channel receiver for incoming log records.
    /// * `shutdown_rx` - Channel receiver carrying the shutdown
    ///   signal, sent by [`FemtoLogger::drop`].
    fn worker_thread_loop(rx: Receiver<QueuedRecord>, shutdown_rx: Receiver<()>) {
        loop {
            // Phase 1: non-blocking check prevents shutdown starvation
            // when the record channel is continuously saturated.
            if Self::should_shutdown_now(&shutdown_rx) {
                Self::shutdown_and_drain(&rx);
                break;
            }
            // Phase 2: block until either a shutdown signal or a new
            // record arrives.  Under heavy load the random selection
            // in select! may repeatedly favour the record branch, so
            // Phase 1 above provides the deterministic guarantee.
            select! {
                recv(shutdown_rx) -> _ => {
                    Self::shutdown_and_drain(&rx);
                    break;
                },
                recv(rx) -> rec => match rec {
                    Ok(job) => Self::handle_log_record(job),
                    Err(_) => break,
                },
            }
        }
    }
}

fn log_join_result(handle: JoinHandle<()>) {
    if handle.join().is_err() {
        warn!("FemtoLogger: worker thread panicked");
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
            Python::attach(|py| py.detach(move || log_join_result(handle)));
        }
    }
}

#[cfg(test)]
#[path = "logger_tests.rs"]
mod logger_tests;
