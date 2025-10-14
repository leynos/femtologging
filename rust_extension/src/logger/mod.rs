//! Core logger implementation for the FemtoLogger system.
//!
//! This module provides the [`FemtoLogger`] struct which handles log message
//! filtering, formatting, and asynchronous output via a background thread.

mod py_handler;
mod worker;

pub use worker::QueuedRecord;

// FIXME(#PyO3/pyo3#3646): Track PyO3 issue for proper fix
use pyo3::prelude::*;
use pyo3::{Py, PyAny};

use crate::filters::FemtoFilter;
use crate::handler::FemtoHandlerTrait;
use crate::manager;
use crate::rate_limited_warner::RateLimitedWarner;

use crate::{
    formatter::{DefaultFormatter, FemtoFormatter},
    level::FemtoLevel,
    log_record::FemtoLogRecord,
};
use crossbeam_channel::Sender;
use log::warn;
// parking_lot avoids poisoning and matches crate-wide locking strategy
use parking_lot::{Mutex, RwLock};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
    Arc,
};
use std::thread::JoinHandle;

use py_handler::{validate_handler, PyHandler};

#[cfg(test)]
use crossbeam_channel::Receiver;

/// Basic logger used for early experimentation.
#[pyclass]
pub struct FemtoLogger {
    /// Identifier used to distinguish log messages from different loggers.
    name: String,
    /// Parent logger name for dotted hierarchy.
    #[pyo3(get)]
    parent: Option<String>,
    formatter: Arc<dyn FemtoFormatter>,
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
    pub fn new(name: String) -> Self {
        Self::with_parent(name, None)
    }

    /// Format a message at the provided level and return it.
    ///
    /// This method currently builds a simple string combining the logger's
    /// name with the level and message.
    #[pyo3(text_signature = "(self, level, message)")]
    pub fn log(&self, level: FemtoLevel, message: &str) -> Option<String> {
        let threshold = self.level.load(Ordering::Relaxed);
        if (level as u8) < threshold {
            return None;
        }
        let record = FemtoLogRecord::new(&self.name, &level.to_string(), message);
        if !self.passes_all_filters(&record) {
            return None;
        }
        let msg = self.formatter.format(&record);
        self.dispatch_to_handlers(record);
        Some(msg)
    }

    /// Update the logger's minimum level.
    ///
    /// `level` accepts "TRACE", "DEBUG", "INFO", "WARN", "ERROR", or
    /// "CRITICAL". The update is thread‑safe because the level is stored in an
    /// `AtomicU8`.
    #[pyo3(text_signature = "(self, level)")]
    pub fn set_level(&self, level: FemtoLevel) {
        self.level.store(level as u8, Ordering::Relaxed);
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
        Python::with_gil(|py| {
            let obj = handler.bind(py);
            validate_handler(obj)?;
            self.add_handler(Arc::new(PyHandler { obj: handler }) as Arc<dyn FemtoHandlerTrait>);
            Ok(())
        })
    }

    /// Remove a handler that was previously attached via `add_handler`.
    #[pyo3(name = "remove_handler", text_signature = "(self, handler)")]
    pub fn py_remove_handler(&self, handler: Py<PyAny>) -> bool {
        Python::with_gil(|py| {
            let mut handlers = self.handlers.write();
            if let Some(pos) = handlers.iter().position(|h| {
                if let Some(py_h) = h.as_any().downcast_ref::<PyHandler>() {
                    py_h.obj.bind(py).is(handler.bind(py))
                } else {
                    false
                }
            }) {
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

    fn handler_ptrs_for_test(&self) -> Vec<usize> {
        self.handlers
            .read()
            .iter()
            .map(|h| Arc::as_ptr(h) as *const () as usize)
            .collect()
    }
}

impl FemtoLogger {
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
        Python::with_gil(|py| {
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
        let formatter: Arc<dyn FemtoFormatter> = Arc::new(DefaultFormatter);
        let handlers: Arc<RwLock<Vec<Arc<dyn FemtoHandlerTrait>>>> =
            Arc::new(RwLock::new(Vec::new()));
        let filters: Arc<RwLock<Vec<Arc<dyn FemtoFilter>>>> = Arc::new(RwLock::new(Vec::new()));

        let worker::WorkerParts {
            tx,
            shutdown_tx,
            handle,
        } = worker::spawn_worker();

        Self {
            name,
            parent,
            formatter,
            level: AtomicU8::new(FemtoLevel::Info as u8),
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
    #[cfg(test)]
    fn handle_log_record(job: QueuedRecord) {
        worker::handle_log_record(job);
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
    /// * `handlers` - Shared collection of handlers used to process records.
    #[cfg(test)]
    fn drain_remaining_records(rx: &Receiver<QueuedRecord>) {
        worker::drain_remaining_records(rx);
    }

    /// Main loop executed by the logger's worker thread.
    ///
    /// Waits on either incoming log records or a shutdown signal using
    /// `select!`. Each received record is forwarded to `handle_log_record`.
    /// When a shutdown signal arrives, any queued records are drained before
    /// the thread exits.
    ///
    /// # Arguments
    ///
    /// * `rx` - Channel receiver for new log records.
    /// * `shutdown_rx` - Channel receiver signaling shutdown.
    /// * `handlers` - Shared collection of handlers for processing records.
    #[cfg(test)]
    fn worker_thread_loop(rx: Receiver<QueuedRecord>, shutdown_rx: Receiver<()>) {
        worker::worker_thread_loop(rx, shutdown_rx);
    }
}

impl Drop for FemtoLogger {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.tx.take();
        if let Some(handle) = self.handle.lock().take() {
            Python::with_gil(|py| {
                py.allow_threads(move || {
                    if handle.join().is_err() {
                        warn!("FemtoLogger: worker thread panicked");
                    }
                })
            });
        }
    }
}
#[cfg(test)]
#[path = "../logger_tests.rs"]
mod logger_tests;
