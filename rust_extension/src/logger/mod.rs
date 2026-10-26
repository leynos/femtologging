//! Core [`FemtoLogger`] implementation for filtering and asynchronous dispatch
//! through the process-wide shard pool, including rebinding after `fork`.
#![allow(
    clippy::too_many_arguments,
    reason = "PyO3 macro-generated wrappers expand Python-call signatures"
)]

#[cfg(feature = "python")]
mod context_snapshot;
mod convenience_methods;
mod dispatcher;
mod flush;
mod flush_diagnostics;
#[cfg(not(loom))]
mod pid_reservation;
mod producer;
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
use crate::rate_limited_warner::RateLimitedWarner;
#[cfg(feature = "python")]
use context_snapshot::ContextSnapshotProvider;

#[cfg(all(test, not(loom)))]
use crate::log_record::FemtoLogRecord;
use crate::{formatter::SharedFormatter, level::FemtoLevel};
// The seam resolves to parking_lot, which avoids poisoning and matches the
// crate-wide locking strategy, outside `--cfg loom`.
use crate::sync::RwLock;
use flush_diagnostics::FlushDiagnostics;
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
#[cfg(all(feature = "python", test, not(loom)))]
pub(crate) use python_helpers::should_capture_exc_info;
pub use queue::QueuedRecord;
pub(crate) use queue::{HandlerAttachment, HandlerRecordSnapshot};

/// Maximum time one flush barrier receives for its Rust-side work, in milliseconds.
const LOGGER_FLUSH_TIMEOUT_MS: u64 = 2_000;

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
    /// Lock-protected live handlers; each queued record owns the snapshot it captured.
    handlers: Arc<RwLock<Vec<HandlerAttachment>>>,
    /// Filters applied only to calls made through this logger before dispatch.
    filters: Arc<RwLock<Vec<Arc<dyn FemtoFilter>>>>,
    /// Number of records refused because the dispatch budget was exhausted.
    dropped_records: AtomicU64,
    /// Number of records dropped because Rust context payload validation failed.
    context_dropped_records: AtomicU64,
    /// Fixed-cardinality accounting for flush barrier outcomes and durations.
    flush_diagnostics: FlushDiagnostics,
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

#[allow(
    clippy::too_many_arguments,
    reason = "PyO3 generates Python-exposed wrappers and signature shims with many parameters"
)]
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

    /// Return the number of records refused at dispatch admission.
    ///
    /// Counts exhausted logger allowance or shard permits, a full or
    /// disconnected shard queue, and a failed replacement-pool start after
    /// `fork`.
    ///
    /// Useful for tests and monitoring dashboards.
    #[pyo3(text_signature = "(self)")]
    pub fn get_dropped(&self) -> u64 {
        self.dropped_records.load(Ordering::Relaxed)
    }

    /// Return how many records were dropped because Python context capture failed.
    #[cfg(feature = "python")]
    #[pyo3(text_signature = "(self)")]
    pub fn get_context_capture_failures(&self) -> u64 {
        self.context_capture_failures.load(Ordering::Relaxed)
    }

    /// Flush all handlers attached to this logger, releasing the GIL while waiting.
    ///
    /// Captures attached handlers and queues a marker on the dispatch shard;
    /// the worker flushes that snapshot before acknowledging the marker.
    /// One 2 second deadline covers the snapshot, marker, acknowledgement, and
    /// every handler flush. Built-in handlers retain shorter configured
    /// timeouts. Releasing the GIL lets Python worker handlers make progress.
    ///
    /// Legacy Rust handlers that implement only synchronous ``flush()`` fail
    /// closed; arbitrary synchronous handler code cannot be preempted.
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` when all stages succeed before the deadline. ``False``
    ///     when completion is unconfirmed or a handler fails.
    ///
    /// Examples
    /// --------
    /// >>> logger.flush_handlers()
    /// True
    #[pyo3(name = "flush_handlers", text_signature = "(self)")]
    fn py_flush_handlers(&self, py: Python<'_>) -> bool {
        self.flush_handlers_detached(py)
    }

    /// Return handler identities so tests can detect replacement or reordering.
    fn handler_ptrs_for_test(&self) -> Vec<usize> {
        self.handlers
            .read()
            .iter()
            .map(|attachment| Arc::as_ptr(&attachment.handler) as *const () as usize)
            .collect()
    }
}

impl FemtoLogger {
    /// Flush all handlers attached to this logger.
    ///
    /// Shares one deadline across the snapshot, shard marker, and handler
    /// flushes. Python entry points release the GIL; native callers do not.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use _femtologging_rs::FemtoLogger;
    ///
    /// let logger = FemtoLogger::new("example".into()).expect("logger starts");
    /// let _ = logger.flush_handlers();
    /// ```
    pub fn flush_handlers(&self) -> bool {
        self.flush_handlers_blocking().finish(
            std::time::Instant::now(),
            &self.flush_diagnostics,
            flush::FlushFailureStage::CallerReturnDeadlineExceeded,
        )
    }

    /// Attach a handler.
    ///
    /// # Errors
    ///
    /// Returns [`crate::handler::HandlerError::MissingContextDispatch`] when a
    /// Python-backed handler lacks contextual dispatch support.
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

#[cfg(all(test, not(loom)))]
mod test_modules;
