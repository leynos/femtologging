//! Core handler trait and error types shared by every handler
//! implementation.

use crate::log_record::FemtoLogRecord;
use crate::monotonic_clock::{SharedClock, system_clock};
use pyo3::prelude::*;
use std::{
    any::Any,
    time::{Duration, Instant},
};
use thiserror::Error;

mod flush_command;

pub(crate) use flush_command::{QueueFlushFailure, flush_command_until};

/// The flush budget of a handler that does not configure one of its own.
///
/// The file handler is the only user: it documents a fixed one-second flush
/// and has no configuration to override it with. The stream, socket and HTTP
/// handlers each hold a configured timeout of their own.
pub(crate) const DEFAULT_FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

/// Time left before `deadline` at `now`, or zero when it has already passed.
///
/// A flush gives each of its waits what remains of one deadline rather than
/// the whole budget afresh, so a deadline in the past yields zero. The timed
/// channel operations treat zero as a non-blocking attempt, not a panic.
///
/// # Examples
///
/// ```ignore
/// let now = Instant::now();
/// let deadline = now + Duration::from_millis(50);
/// assert_eq!(remaining(deadline, now), Duration::from_millis(50));
/// assert_eq!(remaining(now, deadline), Duration::ZERO);
/// ```
pub(crate) fn remaining(deadline: Instant, now: Instant) -> Duration {
    deadline.saturating_duration_since(now)
}

/// Build a deadline from an explicit start time and timeout.
///
/// Callers must fail closed on `None`; falling back to an untimed wait would
/// turn an unrepresentable timeout into an unbounded operation.
///
/// # Examples
///
/// ```ignore
/// let start = Instant::now();
/// let deadline = deadline_after(start, Duration::from_millis(50));
/// assert!(deadline.is_some());
/// assert!(deadline_after(start, Duration::MAX).is_none());
/// ```
pub(crate) fn deadline_after(start: Instant, timeout: Duration) -> Option<Instant> {
    start.checked_add(timeout)
}

/// Return the earlier of a caller's deadline and a local timeout from `now`.
///
/// A handler flush uses this to preserve the caller's absolute deadline while
/// capping its own wait at the timeout that handler documents.
/// If the local timeout cannot be represented by [`Instant`], the caller's
/// deadline remains the effective bound.
///
/// # Examples
///
/// ```ignore
/// let now = Instant::now();
/// let budget = Duration::from_millis(10);
/// let deadline = now + Duration::from_secs(1);
/// assert!(capped_deadline(deadline, now, budget) <= deadline);
/// ```
pub(crate) fn capped_deadline(deadline: Instant, now: Instant, budget: Duration) -> Instant {
    deadline_after(now, budget).map_or(deadline, |local_deadline| deadline.min(local_deadline))
}

/// The deadline and monotonic clock one flush barrier shares with its handlers.
///
/// A barrier resolves one absolute deadline and hands every handler this
/// context rather than a bare [`Instant`]. A handler that samples time through
/// [`Self::now`] therefore observes the same clock the logger used to form the
/// deadline, so a test can drive the whole barrier — admission, worker start,
/// acknowledgement and each handler wait — from one scripted source. Passing
/// `Instant::now` implicitly inside a handler would hide that transition from
/// the logger and make a deadline test depend on the real clock.
///
/// The deadline is optional because an absolute deadline cannot always be
/// represented: `start + timeout` overflows at the far end of [`Instant`]'s
/// range. A context without a deadline reports zero remaining time and is
/// expired, so an implementation fails closed instead of waiting unbounded.
///
/// # Examples
///
/// ```
/// use _femtologging_rs::FlushContext;
/// use std::time::{Duration, Instant};
///
/// let started_at = Instant::now();
/// let context = FlushContext::new(started_at, started_at + Duration::from_secs(2));
///
/// assert_eq!(context.started_at(), started_at);
/// assert!(context.deadline().is_some());
/// // The system clock has advanced since `started_at`, so the budget left is
/// // at most the two seconds the context was given.
/// assert!(context.remaining() <= Duration::from_secs(2));
/// assert!(!context.is_expired());
/// ```
#[derive(Clone)]
pub struct FlushContext {
    /// Time at which this barrier's deadline was formed.
    started_at: Instant,
    /// Absolute end of the barrier budget, or `None` if it overflowed.
    deadline: Option<Instant>,
    /// Monotonic source used to form and sample the deadline.
    clock: SharedClock,
}

impl FlushContext {
    /// Create a context whose deadline is sampled from the system clock.
    ///
    /// Custom handler implementations use this in their own tests. The logger
    /// builds its contexts with the clock the barrier was created with, so a
    /// production flush and a test handler agree on both values.
    ///
    /// # Examples
    ///
    /// ```
    /// use _femtologging_rs::FlushContext;
    /// use std::time::{Duration, Instant};
    ///
    /// let now = Instant::now();
    /// let context = FlushContext::new(now, now + Duration::from_millis(50));
    /// // The system clock has advanced since `now`, so at most the full
    /// // fifty milliseconds remain.
    /// assert!(context.remaining() <= Duration::from_millis(50));
    /// ```
    pub fn new(started_at: Instant, deadline: Instant) -> Self {
        Self::system(started_at, Some(deadline))
    }

    /// Build a context from an explicit clock, including an absent deadline.
    pub(crate) fn with_clock(
        started_at: Instant,
        deadline: Option<Instant>,
        clock: SharedClock,
    ) -> Self {
        Self {
            started_at,
            deadline,
            clock,
        }
    }

    /// Build a context against the production clock.
    pub(crate) fn system(started_at: Instant, deadline: Option<Instant>) -> Self {
        Self::with_clock(started_at, deadline, system_clock())
    }

    /// Return the start time used to form the deadline.
    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    /// Return the absolute deadline, or `None` when it was unrepresentable.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Sample the same monotonic clock used to form this deadline.
    pub fn now(&self) -> Instant {
        self.clock.now()
    }

    /// Return the time remaining before the deadline, or zero once it passes.
    pub fn remaining(&self) -> Duration {
        self.deadline
            .map(|deadline| remaining(deadline, self.now()))
            .unwrap_or(Duration::ZERO)
    }

    /// Whether no positive wait budget remains.
    pub fn is_expired(&self) -> bool {
        self.remaining().is_zero()
    }

    /// Whether the sampled time is strictly after the deadline.
    pub fn is_late(&self) -> bool {
        self.deadline.is_none_or(|deadline| self.now() > deadline)
    }
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

    /// Flush pending records only when the implementation can honour `context`.
    ///
    /// Returning `true` signals the flush completed successfully.
    /// Implementations may return `false` when the handler has been closed or
    /// if the flush command could not be processed.
    ///
    /// An implementation that opts into this method must not block past
    /// [`FlushContext::deadline`], and must not report success after it. It must
    /// sample time through [`FlushContext::now`] rather than [`Instant::now`],
    /// so a barrier test can control the deadline transition. Synchronous
    /// arbitrary handler code cannot be preempted, so this is an implementation
    /// contract. [`FemtoLogger::flush_handlers`] advertises one timeout for the
    /// whole barrier — snapshotting the handler list, enqueueing the marker,
    /// waiting for its acknowledgement, and flushing every handler. Each
    /// built-in implementation still caps its wait at its own configured
    /// timeout, so the deadline can shorten that wait without lengthening it.
    ///
    /// The default fails closed: it returns `false` without calling
    /// [`Self::flush`]. This keeps a legacy synchronous flush from blocking
    /// the logger's caller indefinitely. A custom handler must override this
    /// method to opt into the deadline-aware flush contract; the logger cannot
    /// preempt an implementation that violates that contract.
    ///
    /// The logger calls this method on its worker without holding the
    /// handler-list lock. A handler removed during a flush stays alive in the
    /// queued snapshot until the worker releases it. In a normal build, the
    /// worker belongs to a process-wide pool and logger drop waits at most the
    /// bounded drain timeout for that logger's in-flight records; it does not
    /// join the shared worker. A callback that violates this deadline contract
    /// can keep that worker occupied after the caller returns `false` and after
    /// the logger is dropped. The snapshot keeps the handler alive until the
    /// callback returns. Loom models instead join their logger-owned worker.
    /// Synchronous arbitrary code cannot be preempted.
    ///
    /// A handler whose flush has no observable completion — a user-defined
    /// Python handler, for example — may confirm that the barrier delivered
    /// every record to it without claiming its own buffers were written. Such
    /// a handler must document that narrower promise; the logger only reports
    /// what the handler returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use _femtologging_rs::{FemtoHandlerTrait, FlushContext};
    /// use std::time::{Duration, Instant};
    ///
    /// fn flush_before_deadline(handler: &dyn FemtoHandlerTrait) -> bool {
    ///     let now = Instant::now();
    ///     handler.flush_until(&FlushContext::new(now, now + Duration::from_secs(2)))
    /// }
    /// ```
    ///
    /// [`FemtoLogger::flush_handlers`]: crate::FemtoLogger::flush_handlers
    fn flush_until(&self, _context: &FlushContext) -> bool {
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

    fn flush_until(&self, _context: &FlushContext) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
#[path = "handler_tests.rs"]
mod tests;
