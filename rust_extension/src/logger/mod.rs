//! Core logger implementation for the [`FemtoLogger`] system.
//!
//! This module provides the [`FemtoLogger`] struct which handles log message
//! filtering, formatting, and asynchronous output via a background thread.
mod convenience_methods;
mod producer;
mod py_handler;
mod python_bindings;
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
        Self::remove_attachment(&self.handlers, handler)
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
        Self::remove_attachment(&self.filters, filter)
    }

    /// Remove the first attachment with matching identity, preserving the rest's order.
    fn remove_attachment<T: ?Sized>(attachments: &RwLock<Vec<Arc<T>>>, target: &Arc<T>) -> bool {
        let mut entries = attachments.write();
        entries
            .iter()
            .position(|current| Arc::ptr_eq(current, target))
            .is_some_and(|position| {
                entries.remove(position);
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
