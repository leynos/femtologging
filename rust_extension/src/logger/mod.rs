//! Core logger implementation for the FemtoLogger system.
//!
//! This module provides the [`FemtoLogger`] struct, which handles log message
//! filtering, formatting, and asynchronous output through the process-wide
//! dispatch pool. A logger holds a reference to one shard of that pool rather
//! than a thread of its own, so the threads a process spends on logging follow
//! the machine rather than the number of loggers it has created.
#![allow(
    clippy::too_many_arguments,
    reason = "PyO3 macro-generated wrappers expand Python-call signatures"
)]

mod convenience_methods;
mod dispatcher;
#[cfg(feature = "python")]
mod context_snapshot;
mod producer;
mod py_handler;
mod python_helpers;
#[cfg(feature = "python")]
mod runtime_mutation;
mod worker;

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::{Py, PyAny};
use std::sync::Arc;

use crate::filters::FemtoFilter;
use crate::handler::FemtoHandlerTrait;
use crate::rate_limited_warner::RateLimitedWarner;
#[cfg(feature = "python")]
use context_snapshot::ContextSnapshotProvider;

use crate::{formatter::SharedFormatter, level::FemtoLevel, log_record::FemtoLogRecord};
// The seam resolves to parking_lot, which avoids poisoning and matches the
// crate-wide locking strategy, outside `--cfg loom`.
use crate::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

pub use dispatcher::DispatcherError;
pub(crate) use dispatcher::{LoggerBudget, Shard};

pub use py_handler::{PyHandler, validate_handler};
#[cfg(all(feature = "python", test))]
pub(crate) use python_helpers::should_capture_exc_info;

/// Record queued for dispatch, with the handlers it was logged through.
///
/// The handler list is captured when the record is accepted rather than read
/// when it is handled, so a handler removed in between does not lose records
/// already accepted for it.
pub struct QueuedRecord {
    pub record: FemtoLogRecord,
    pub handlers: Vec<Arc<dyn FemtoHandlerTrait>>,
    #[cfg(feature = "python")]
    pub context: Option<Py<PyAny>>,
}

impl From<DispatcherError> for PyErr {
    /// Report a pool that could not start as a Python exception.
    ///
    /// A process that cannot start threads is out of a resource, so this maps
    /// to `RuntimeError`: the call was well-formed and the environment refused
    /// it. The message names how many shards were started, so an operator can
    /// tell a partly started pool from one that never began.
    fn from(error: DispatcherError) -> Self {
        PyRuntimeError::new_err(error.to_string())
    }
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
    /// Number of records dropped because Rust context payload validation failed.
    context_dropped_records: AtomicU64,
    drop_warner: RateLimitedWarner,
    #[cfg(feature = "python")]
    context_snapshot_provider: Arc<dyn ContextSnapshotProvider>,
    #[cfg(feature = "python")]
    context_capture_failures: AtomicU64,
    #[cfg(feature = "python")]
    context_capture_warner: RateLimitedWarner,
    /// The shard this logger's records are dispatched on.
    ///
    /// A shard, not a thread: the pool is shared, so the number of loggers a
    /// process creates no longer decides how many threads it runs.
    ///
    /// Borrowed from a pool that outlives the process's loggers, or — in a Loom
    /// model, which has no pool — owned by this logger alone.
    #[cfg(not(loom))]
    shard: &'static Shard,
    #[cfg(loom)]
    shard: Arc<Shard>,
    /// How much of the shard's in-flight capacity this logger may hold.
    ///
    /// Per logger, so one logger's slow handler cannot consume the shard's
    /// whole permit pool and starve the loggers sharing it. Shared with the
    /// permits it was charged against, so a record's charge is returned when
    /// its handlers finish even if this logger has already been dropped.
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
    /// and no ASCII control characters. A logger built this way is not
    /// registered in the global manager, so it does not count against the
    /// registry limit.
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

    /// Return how many records were dropped because Python context capture failed.
    #[cfg(feature = "python")]
    #[pyo3(text_signature = "(self)")]
    pub fn get_context_capture_failures(&self) -> u64 {
        self.context_capture_failures.load(Ordering::Relaxed)
    }

    /// Flush all handlers attached to this logger.
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
    ///
    /// # Errors
    ///
    /// Returns [`crate::handler::HandlerError::MissingContextDispatch`] if a Python-backed
    /// handler does not provide contextual dispatch support.
    pub fn add_handler(
        &self,
        handler: Arc<dyn FemtoHandlerTrait>,
    ) -> Result<(), crate::handler::HandlerError> {
        #[cfg(feature = "python")]
        if handler.is_python_backed() && !handler.provides_context_dispatch() {
            return Err(crate::handler::HandlerError::MissingContextDispatch);
        }
        self.handlers.write().push(handler);
        Ok(())
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
        if self.budget.in_flight() > 0 {
            // The wait is taken while detached from the Python interpreter,
            // because a handler on the shard may be a Python object that needs
            // the GIL to finish and holding it here would deadlock the two.
            // When there is no interpreter, no Python handler can be mid-call
            // either, so the wait is taken directly.
            if Python::try_attach(|py| py.detach(|| self.shard.wait_for_drain())).is_none() {
                let _ = self.shard.wait_for_drain();
            }
        }
    }
}

// These tests drive the worker with real threads and `crossbeam_channel`, so
// they build outside `--cfg loom` only; the heavy lane's models cover Loom.
#[cfg(all(test, not(loom)))]
#[path = "logger_tests.rs"]
mod logger_tests;
#[cfg(all(test, not(loom)))]
#[path = "logger_tests_helpers.rs"]
mod logger_tests_helpers;
#[cfg(all(test, not(loom)))]
#[path = "producer_tests.rs"]
mod producer_tests;
#[cfg(all(test, not(loom), feature = "python"))]
#[path = "producer_context_tests.rs"]
mod producer_context_tests;
