//! Core logger implementation for the FemtoLogger system.
//!
//! This module provides the [`FemtoLogger`] struct, which handles log message
//! filtering, formatting, and asynchronous output through the process-wide
//! dispatch pool. A logger uses one shard of that pool rather than owning a
//! thread, so the threads a process spends on logging follow the machine rather
//! than the number of loggers it has created. If a logger crosses a `fork`, it
//! rebinds to the child process's pool before its next dispatch.
#![allow(
    clippy::too_many_arguments,
    reason = "PyO3 macro-generated wrappers expand Python-call signatures"
)]

#[cfg(feature = "python")]
mod context_snapshot;
mod convenience_methods;
mod dispatcher;
#[cfg(not(loom))]
mod pid_reservation;
mod producer;
mod python_api;
mod py_handler;
mod python_helpers;
mod queue;
#[cfg(feature = "python")]
mod runtime_mutation;
mod worker;

use pyo3::prelude::*;
use std::sync::Arc;

use crate::filters::FemtoFilter;
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::rate_limited_warner::RateLimitedWarner;
#[cfg(feature = "python")]
use context_snapshot::ContextSnapshotProvider;

#[cfg(all(test, not(loom)))]
use crate::log_record::FemtoLogRecord;
use crate::formatter::SharedFormatter;
#[cfg(test)]
use crate::level::FemtoLevel;
// The seam resolves to parking_lot, which avoids poisoning and matches the
// crate-wide locking strategy, outside `--cfg loom`.
use crate::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

pub use dispatcher::DispatcherError;
#[cfg(all(
    test,
    not(loom),
    any(feature = "log-compat", feature = "tracing-compat")
))]
pub(crate) use dispatcher::worker_count_for_test;
pub(crate) use dispatcher::{LoggerBudget, Shard};

pub use py_handler::{PyHandler, validate_handler};
#[cfg(all(feature = "python", test))]
pub(crate) use python_helpers::should_capture_exc_info;
pub use queue::QueuedRecord;
pub(crate) use queue::{HandlerAttachment, HandlerRecordSnapshot};

/// Basic logger used for early experimentation.
#[pyclass]
pub struct FemtoLogger {
    /// Identifier used to distinguish log messages from different loggers.
    name: String,
    /// Parent logger name for dotted hierarchy.
    #[pyo3(get)]
    parent: Option<String>,
    /// Formatter shared by records emitted through this logger.
    formatter: SharedFormatter,
    /// Current severity threshold, stored atomically for concurrent logging.
    level: AtomicU8,
    /// Whether accepted records are also sent to the parent logger.
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
    /// Number of records refused because the dispatch budget was exhausted.
    dropped_records: AtomicU64,
    /// Number of records dropped because Rust context payload validation failed.
    context_dropped_records: AtomicU64,
    /// Rate limiter for warnings about refused dispatch records.
    drop_warner: RateLimitedWarner,
    /// Captures Python context for queued handlers when they need it.
    #[cfg(feature = "python")]
    context_snapshot_provider: Arc<dyn ContextSnapshotProvider>,
    /// Number of attempts that failed while capturing Python context.
    #[cfg(feature = "python")]
    context_capture_failures: AtomicU64,
    /// Rate limiter for warnings about failed context captures.
    #[cfg(feature = "python")]
    context_capture_warner: RateLimitedWarner,
    #[cfg(not(loom))]
    /// Inline process-specific shard state, refreshed after a fork.
    dispatch: worker::DispatchState,
    #[cfg(loom)]
    /// A Loom model's shard is owned by the logger and shut down on drop.
    shard: Arc<Shard>,
    /// In-flight allowance used by this logger in its Loom model.
    ///
    /// Ordinary builds keep the budget with the shard in [`DispatchState`],
    /// so the two can be replaced together after a fork.
    #[cfg(loom)]
    budget: Arc<LoggerBudget>,
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
    /// Returns [`crate::handler::HandlerError::MissingContextDispatch`] if a Python-backed
    /// handler does not provide contextual dispatch support.
    pub fn add_handler(
        &self,
        handler: Arc<dyn FemtoHandlerTrait>,
    ) -> Result<(), crate::handler::HandlerError> {
        self.add_handler_attachment(HandlerAttachment::new(handler))
    }

    /// Attach a handler with filters evaluated on the producer thread.
    #[cfg(all(test, feature = "python"))]
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
}

impl Drop for FemtoLogger {
    fn drop(&mut self) {
        // A model's shard is the logger's own, so the drop is what ends the
        // model thread and, by joining it, what guarantees the queued records
        // were handled.
        #[cfg(loom)]
        self.shard.shut_down();
        // A pool shard's worker is shared and stays, so the drop waits for what
        // *this* logger queued — and only then. Waiting unconditionally would
        // make every logger's drop wait on the whole shard's backlog, including
        // other loggers' work: dropping ten thousand loggers at once would pay
        // the barrier timeout ten thousand times over, each while holding the
        // registry lock that is dropping them. A logger with nothing in flight
        // has nothing of its own to drain.
        #[cfg(not(loom))]
        self.finish_dispatch_state();
    }
}

#[cfg(test)]
mod test_modules;
