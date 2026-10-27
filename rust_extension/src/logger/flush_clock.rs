//! The monotonic time source and absolute deadline shared by one flush barrier.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::handler::{deadline_after, remaining};

/// Monotonic clock used to sample one flush barrier's deadline checks.
pub(super) trait MonotonicClock: Send + Sync {
    /// Return the current time in the clock's monotonic domain.
    fn now(&self) -> Instant;
}

/// Production clock backed by `Instant`.
pub(super) struct SystemMonotonicClock;

impl MonotonicClock for SystemMonotonicClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Clock and absolute deadline carried through flush orchestration.
///
/// Timed locks and channels receive durations sampled from this context. The
/// production clock and those blocking primitives use the same `Instant`
/// domain; tests can supply a clock to exercise deadline transitions.
#[derive(Clone)]
pub(crate) struct FlushDeadline {
    /// Time at which this barrier's deadline was created.
    started_at: Instant,
    /// Absolute end of the barrier budget, or `None` if it overflowed.
    deadline: Option<Instant>,
    /// Monotonic source used to form and sample the deadline.
    clock: Arc<dyn MonotonicClock>,
}

impl FlushDeadline {
    /// Create a barrier deadline from one clock sample.
    pub(super) fn with_clock(timeout: Duration, clock: Arc<dyn MonotonicClock>) -> Self {
        let started_at = clock.now();
        let deadline = deadline_after(started_at, timeout);
        Self {
            started_at,
            deadline,
            clock,
        }
    }

    /// Create a barrier deadline using the production monotonic clock.
    pub(super) fn system(timeout: Duration) -> Self {
        Self::with_clock(timeout, Arc::new(SystemMonotonicClock))
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
}
