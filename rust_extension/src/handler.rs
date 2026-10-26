//! Core handler trait and error types shared by every handler
//! implementation.

use crate::log_record::FemtoLogRecord;
use pyo3::prelude::*;
use std::{
    any::Any,
    time::{Duration, Instant},
};
use thiserror::Error;

/// The flush budget of a handler that does not configure one of its own.
///
/// The file handler is the only user: it documents a fixed one-second flush
/// and has no configuration to override it with. The stream, socket and HTTP
/// handlers each hold a configured timeout of their own.
pub(crate) const DEFAULT_FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

/// Time left before `deadline`, or zero when it has already passed.
///
/// A flush gives each of its waits what remains of one deadline rather than
/// the whole budget afresh, so a deadline in the past yields zero. The timed
/// channel operations treat zero as a non-blocking attempt, not a panic.
///
/// # Examples
///
/// ```ignore
/// let deadline = Instant::now() + Duration::from_millis(50);
/// assert!(remaining(deadline) <= Duration::from_millis(50));
/// assert_eq!(remaining(Instant::now()), Duration::ZERO);
/// ```
pub(crate) fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

/// Return the earlier of a caller's deadline and a local timeout measured now.
///
/// A handler flush uses this to preserve the caller's absolute deadline while
/// capping its own wait at the timeout that handler documents.
///
/// # Examples
///
/// ```ignore
/// let budget = Duration::from_millis(10);
/// let deadline = Instant::now() + Duration::from_secs(1);
/// assert!(capped_deadline(deadline, budget) <= deadline);
/// ```
pub(crate) fn capped_deadline(deadline: Instant, budget: Duration) -> Instant {
    deadline.min(Instant::now() + budget)
}

/// Errors reported by handler implementations when dispatching a log record.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HandlerError {
    /// The handler's queue rejected the record because it was already full.
    #[error("queue full")]
    QueueFull,
    /// The handler is no longer accepting records because it has been closed.
    #[error("handler is closed")]
    Closed,
    /// A Python-backed handler did not opt in to contextual dispatch.
    #[error("Python-backed handlers must provide contextual dispatch")]
    MissingContextDispatch,
    /// Sending the record timed out before the handler could accept it.
    #[error("handler send timed out after {0:?}")]
    Timeout(Duration),
    /// Catch-all variant for handler specific failures.
    #[error("{0}")]
    Message(String),
}

impl From<HandlerError> for PyErr {
    fn from(error: HandlerError) -> Self {
        pyo3::exceptions::PyRuntimeError::new_err(error.to_string())
    }
}

/// Trait implemented by all log handlers.
///
/// `FemtoHandler` is `Send + Sync` so it can be safely called from multiple
/// threads by reference. Each implementation forwards the record to its own
/// consumer thread without blocking the caller.
pub trait FemtoHandlerTrait: Send + Sync + Any {
    /// Dispatch a log record for handling.
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError>;

    /// Return whether this handler invokes Python while dispatching records.
    ///
    /// Python-backed handlers must override [`Self::handle_with_context`] to
    /// use the supplied context and return `true` from
    /// [`Self::provides_context_dispatch`] to declare that capability.
    fn is_python_backed(&self) -> bool {
        false
    }

    /// Report whether this handler provides contextual Python dispatch.
    ///
    /// Python-backed implementations must return `true` only when they
    /// override [`Self::handle_with_context`] and invoke Python inside the
    /// supplied context.
    #[cfg(feature = "python")]
    fn provides_context_dispatch(&self) -> bool {
        false
    }

    /// Dispatch a record inside a captured Python context when applicable.
    ///
    /// The default preserves direct dispatch for native handlers. Python-backed
    /// handlers must override this method to invoke Python inside the supplied
    /// context and return `true` from [`Self::provides_context_dispatch`]; the
    /// default fails closed rather than silently discarding captured context.
    #[cfg(feature = "python")]
    fn handle_with_context(
        &self,
        record: FemtoLogRecord,
        _context: Option<&Py<PyAny>>,
    ) -> Result<(), HandlerError> {
        if self.is_python_backed() {
            Err(HandlerError::MissingContextDispatch)
        } else {
            self.handle(record)
        }
    }

    /// Flush any pending log records.
    ///
    /// Returning `true` signals the flush completed successfully.
    /// Implementations may return `false` when the handler has been closed or
    /// if the flush command could not be processed.
    ///
    fn flush(&self) -> bool {
        // Default to a no-op flush for handlers that do not buffer writes.
        true
    }

    /// Flush pending records only when the implementation can honour `deadline`.
    ///
    /// Returning `true` signals the flush completed successfully.
    /// Implementations may return `false` when the handler has been closed or
    /// if the flush command could not be processed.
    ///
    /// An implementation that opts into this method must not block past
    /// `deadline`, and must not report success after it. Synchronous arbitrary
    /// handler code cannot be preempted, so this is an implementation contract.
    /// [`FemtoLogger::flush_handlers`] advertises one timeout for the whole
    /// barrier — snapshotting the handler list, enqueueing the marker, waiting
    /// for its acknowledgement, and flushing every handler. Each built-in
    /// implementation still caps its wait at its own configured timeout, so
    /// the deadline can shorten that wait without lengthening it.
    ///
    /// The default fails closed: it returns `false` without calling
    /// [`Self::flush`]. This keeps a legacy synchronous flush from blocking
    /// the logger's caller indefinitely. A custom handler must override this
    /// method to opt into the deadline-aware flush contract; the logger cannot
    /// preempt an implementation that violates that contract.
    ///
    /// The logger calls this method on its worker without holding the
    /// handler-list lock. A handler may be removed while a flush is in
    /// progress; the queued marker keeps it alive until the worker releases
    /// the snapshot. The `flush_handlers()` caller waits only until its shared
    /// deadline, but logger shutdown joins the worker and therefore waits for
    /// this callback to return. Synchronous arbitrary code cannot be
    /// preempted.
    ///
    /// # Examples
    ///
    /// ```
    /// use _femtologging_rs::FemtoHandlerTrait;
    /// use std::time::{Duration, Instant};
    ///
    /// fn flush_before_deadline(handler: &dyn FemtoHandlerTrait) -> bool {
    ///     handler.flush_until(Instant::now() + Duration::from_secs(2))
    /// }
    /// ```
    ///
    /// [`FemtoLogger::flush_handlers`]: crate::FemtoLogger::flush_handlers
    fn flush_until(&self, _deadline: Instant) -> bool {
        false
    }

    /// Expose a typed reference for downcasting.
    fn as_any(&self) -> &dyn Any;
}

/// Base Python class for handlers. Methods do nothing by default.
#[pyclass(name = "FemtoHandler", subclass)]
#[derive(Default)]
pub struct FemtoHandler;

#[pymethods]
impl FemtoHandler {
    /// Creates the empty Python-facing marker handler; its default methods intentionally accept records and flush requests without side effects.
    #[new]
    fn py_new() -> Self {
        Self
    }
}

impl FemtoHandlerTrait for FemtoHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush(&self) -> bool {
        true
    }

    fn flush_until(&self, _deadline: Instant) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
