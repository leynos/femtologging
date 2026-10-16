//! Core logger implementation for the FemtoLogger system.
//!
//! This module provides the [`FemtoLogger`] struct which handles log message
//! filtering, formatting, and asynchronous output via a background thread.
#![allow(
    clippy::too_many_arguments,
    reason = "PyO3 macro-generated wrappers expand Python-call signatures"
)]

#[cfg(feature = "python")]
mod context_snapshot;
mod convenience_methods;
mod producer;
mod python_api;
mod py_handler;
mod python_helpers;
mod queue;
#[cfg(feature = "python")]
mod runtime_mutation;
mod worker;

use pyo3::prelude::*;
use pyo3::{Py, PyAny};
use std::sync::Arc;

use crate::filters::FemtoFilter;
use crate::handler::{FemtoHandlerTrait, HandlerError};
#[cfg(all(test, not(loom)))]
use crate::log_record::FemtoLogRecord;
use crate::rate_limited_warner::RateLimitedWarner;
#[cfg(feature = "python")]
use context_snapshot::ContextSnapshotProvider;

use crate::{formatter::SharedFormatter, level::FemtoLevel};
// The seam resolves to parking_lot, which avoids poisoning and matches the
// crate-wide locking strategy, outside `--cfg loom`.
use crate::sync::{JoinHandle, Mutex, RwLock, Sender};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

pub use py_handler::{PyHandler, validate_handler};
#[cfg(all(feature = "python", test))]
pub(crate) use python_helpers::should_capture_exc_info;
pub use queue::QueuedRecord;
pub(crate) use queue::{FlushAckHandler, HandlerAttachment, HandlerRecordSnapshot};

/// Maximum number of records held before producers report a dropped record.
const DEFAULT_CHANNEL_CAPACITY: usize = 1024;
/// Maximum time a flush waits for the worker's acknowledgement, in milliseconds.
const LOGGER_FLUSH_TIMEOUT_MS: u64 = 2_000;

/// Basic logger used for early experimentation.
#[pyclass]
pub struct FemtoLogger {
    /// Identifier used to distinguish log messages from different loggers.
    name: String,
    /// Parent logger name for dotted hierarchy.
    #[pyo3(get)]
    parent: Option<String>,
    /// Shared formatter used by producer threads before records enter the queue.
    formatter: SharedFormatter,
    /// Atomically readable level threshold updated by logger configuration calls.
    level: AtomicU8,
    /// Whether records are copied to the dotted-name parent after local dispatch.
    propagate: AtomicBool,
    /// Whether configuration has disabled this logger.
    ///
    /// Only the emitting logger checks this flag, so a disabled logger rejects
    /// its own records without changing how an *enabled* logger's ancestors
    /// dispatch. `disable_existing_loggers` relies on it to stop omitted
    /// loggers reaching ancestor handlers through propagation.
    disabled: AtomicBool,
    /// Lock-protected live handlers; each queued record owns the snapshot it captured.
    handlers: Arc<RwLock<Vec<HandlerAttachment>>>,
    /// Filters applied only to calls made through this logger before dispatch.
    filters: Arc<RwLock<Vec<Arc<dyn FemtoFilter>>>>,
    /// Number of records rejected because the worker queue could not accept them.
    dropped_records: AtomicU64,
    /// Number of records dropped because Python context capture failed.
    context_dropped_records: AtomicU64,
    /// Rate-limits warnings emitted for queue drops across concurrent producers.
    drop_warner: RateLimitedWarner,
    /// Producer-owned provider used to snapshot Python context for queued handlers.
    #[cfg(feature = "python")]
    context_snapshot_provider: Arc<dyn ContextSnapshotProvider>,
    /// Counts records rejected when capturing producer Python context fails.
    #[cfg(feature = "python")]
    context_capture_failures: AtomicU64,
    /// Rate-limits warnings for context-capture failures independently of queue drops.
    #[cfg(feature = "python")]
    context_capture_warner: RateLimitedWarner,
    /// Producer endpoint for records; taking it during drop prevents new sends.
    tx: Option<Sender<QueuedRecord>>,
    /// Worker shutdown signal sent before the worker join during logger drop.
    shutdown_tx: Option<Sender<()>>,
    /// Mutex-protected worker handle so dropping the logger can join without a lock held.
    handle: Mutex<Option<JoinHandle<()>>>,
}


impl FemtoLogger {
    /// Flush all handlers attached to this logger.
    ///
    /// First waits up to 2 seconds for the internal worker thread to drain
    /// its queue, then calls ``flush()`` on every attached handler (each
    /// handler applies its own timeout).
    pub fn flush_handlers(&self) -> bool {
        self.flush_handlers_blocking()
    }

    /// Attach a handler to this logger.
    ///
    /// # Errors
    ///
    /// Returns [`HandlerError::MissingContextDispatch`] when a Python-backed
    /// handler does not declare contextual dispatch support.
    pub fn add_handler(
        &self,
        handler: Arc<dyn FemtoHandlerTrait>,
    ) -> Result<(), crate::handler::HandlerError> {
        self.add_handler_attachment(HandlerAttachment::new(handler))
    }

    /// Attach a handler with filters evaluated on the producer thread.
    #[cfg(feature = "python")]
    pub(crate) fn add_handler_with_filters(
        &self,
        handler: Arc<dyn FemtoHandlerTrait>,
        filters: Vec<Arc<dyn FemtoFilter>>,
    ) -> Result<(), HandlerError> {
        self.add_handler_attachment(HandlerAttachment::with_filters(handler, filters))
    }

    /// Validate contextual dispatch support before publishing a handler attachment.
    fn add_handler_attachment(&self, attachment: HandlerAttachment) -> Result<(), HandlerError> {
        #[cfg(feature = "python")]
        if attachment.handler.is_python_backed() && !attachment.handler.provides_context_dispatch()
        {
            return Err(HandlerError::MissingContextDispatch);
        }
        self.handlers.write().push(attachment);
        Ok(())
    }

    /// Attach a filter to this logger.
    pub fn add_filter(&self, filter: Arc<dyn FemtoFilter>) {
        self.filters.write().push(filter);
    }

    /// Detach a handler previously added to this logger.
    pub fn remove_handler(&self, handler: &Arc<dyn FemtoHandlerTrait>) -> bool {
        let mut handlers = self.handlers.write();
        if let Some(pos) = handlers
            .iter()
            .position(|attachment| Arc::ptr_eq(&attachment.handler, handler))
        {
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

    /// Return whether configuration has disabled this logger.
    ///
    /// Only the emitting logger checks this flag; [`Self::dispatch_to_handlers`]
    /// deliberately does not, so an *enabled* logger still reaches ancestor
    /// handlers after propagation.
    pub fn is_disabled(&self) -> bool {
        self.disabled.load(Ordering::SeqCst)
    }

    /// Set whether configuration disables this logger.
    ///
    /// Sequentially consistent, matching the propagation flag. Records already
    /// snapshotted by [`Self::send_to_local_handlers`] are not retracted, as
    /// with [`Self::clear_handlers`].
    pub(crate) fn set_disabled(&self, flag: bool) {
        self.disabled.store(flag, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub fn handlers_for_test(&self) -> Vec<Arc<dyn FemtoHandlerTrait>> {
        self.handlers
            .read()
            .iter()
            .map(|attachment| attachment.handler.clone())
            .collect()
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
            worker::join_worker(handle);
        }
    }
}
// These real-thread tests use crossbeam outside Loom; the heavy lane uses models.
#[cfg(all(test, not(loom)))]
#[path = "logger_tests.rs"]
mod logger_tests;
#[cfg(all(test, not(loom)))]
#[path = "logger_tests_helpers.rs"]
mod logger_tests_helpers;
#[cfg(all(test, not(loom)))]
#[path = "producer_rejection_tests.rs"]
mod producer_rejection_tests;
#[cfg(all(test, not(loom)))]
#[path = "producer_tests.rs"]
mod producer_tests;
#[cfg(all(test, not(loom), feature = "tracing-compat"))]
#[path = "producer_tracing_tests.rs"]
mod producer_tracing_tests;
#[cfg(all(test, not(loom)))]
#[path = "worker_tests.rs"]
mod worker_tests;
// weave: run 'weave explain rust_extension/src/logger/mod.rs' for per-hunk detail, 'weave check' to verify your resolution
