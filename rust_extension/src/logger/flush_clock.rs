//! The monotonic time source and absolute deadline shared by one flush barrier.

use std::time::{Duration, Instant};

use crate::handler::{FlushContext, deadline_after, remaining};
use crate::monotonic_clock::SharedClock;

/// Clock and absolute deadline carried through flush orchestration.
///
/// Timed locks and channels receive durations sampled from this context. The
/// production clock and those blocking primitives use the same `Instant`
/// domain; tests can supply a clock to exercise deadline transitions. The
/// context also carries the barrier's trace span and tracing dispatcher, so the
/// worker draining the marker stays inside the span the caller opened and
/// reports through the subscriber the caller is using — including a
/// thread-local one that the worker would otherwise not see.
#[derive(Clone)]
pub(crate) struct FlushDeadline {
    /// Time at which this barrier's deadline was created.
    started_at: Instant,
    /// Absolute end of the barrier budget, or `None` if it overflowed.
    deadline: Option<Instant>,
    /// Monotonic source used to form and sample the deadline.
    clock: SharedClock,
    /// Span linked by the worker's and caller's bounded flush events.
    #[cfg(feature = "tracing-compat")]
    span: tracing::Span,
    /// Dispatcher active when the barrier was created.
    ///
    /// A thread-local subscriber lives only on the calling thread. The worker
    /// installs this dispatcher for the duration of the job, so its events
    /// reach the subscriber that asked for the flush rather than the global
    /// one, or nowhere at all.
    #[cfg(feature = "tracing-compat")]
    dispatcher: tracing::Dispatch,
}

impl FlushDeadline {
    /// Create a barrier deadline from one clock sample.
    pub(super) fn with_clock(timeout: Duration, clock: SharedClock) -> Self {
        let started_at = clock.now();
        let deadline = deadline_after(started_at, timeout);
        Self {
            started_at,
            deadline,
            clock,
            #[cfg(feature = "tracing-compat")]
            span: tracing::trace_span!(
                target: "femtologging::internal",
                "flush_handlers",
                operation = "flush_handlers",
            ),
            #[cfg(feature = "tracing-compat")]
            dispatcher: tracing::dispatcher::get_default(|dispatcher| dispatcher.clone()),
        }
    }

    /// Span to enter while recording this barrier's bounded events.
    #[cfg(feature = "tracing-compat")]
    pub(super) fn span(&self) -> &tracing::Span {
        &self.span
    }

    /// Dispatcher the worker must install before recording this barrier.
    #[cfg(feature = "tracing-compat")]
    pub(super) fn dispatcher(&self) -> &tracing::Dispatch {
        &self.dispatcher
    }

    /// Create a barrier deadline using the production monotonic clock.
    pub(super) fn system(timeout: Duration) -> Self {
        Self::with_clock(timeout, crate::monotonic_clock::system_clock())
    }

    /// Return the start time used to form the deadline.
    pub(super) fn started_at(&self) -> Instant {
        self.started_at
    }

    /// Return the absolute deadline, or `None` when it was unrepresentable.
    pub(super) fn instant(&self) -> Option<Instant> {
        self.deadline
    }

    /// Sample the same monotonic clock used to form this deadline.
    pub(super) fn now(&self) -> Instant {
        self.clock.now()
    }

    /// Return time remaining from the shared deadline.
    pub(super) fn remaining(&self) -> Duration {
        self.deadline
            .map(|deadline| remaining(deadline, self.now()))
            .unwrap_or(Duration::ZERO)
    }

    /// Whether no positive wait budget remains.
    pub(super) fn is_expired(&self) -> bool {
        self.remaining().is_zero()
    }

    /// Whether the sampled time is strictly after the deadline.
    pub(super) fn is_late(&self) -> bool {
        self.deadline.is_none_or(|deadline| self.now() > deadline)
    }

    /// Hand the same deadline and clock to the handlers this barrier flushes.
    pub(super) fn context(&self) -> FlushContext {
        FlushContext::with_clock(self.started_at, self.deadline, self.clock.clone())
    }
}
