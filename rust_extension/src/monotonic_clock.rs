//! The monotonic clock seam the shutdown queries and deadlines read through.
//!
//! [`ShutdownSignal`] answers questions about *when*: whether a drain deadline
//! has passed, how much of the budget is left, and what a configured timeout
//! may be clamped to. Read against `Instant::now` directly, each of those is a
//! query that looks pure and behaves unpredictably: a test can only ask it
//! about the real present, so the boundary between "the deadline has just
//! passed" and "the deadline is a nanosecond away" is a race the test does not
//! control.
//!
//! This module makes the time source explicit. Production samples
//! [`SystemClock`], which is `Instant::now` and nothing else. Tests sample
//! [`ManualClock`], which stands still until they move it, so a query's answer
//! depends only on the phase and the instant the test chose.
//!
//! # A second clock, not the rate-limited warner's
//!
//! [`crate::rate_limited_warner`] already defines a `Clock` seam, and this is
//! deliberately not that one. Its `now_millis` reports milliseconds since the
//! warner was constructed: an opaque counter for deciding whether a warning
//! interval has elapsed. A shutdown deadline is an absolute [`Instant`] that
//! must be compared against another absolute instant, and a converted counter
//! cannot express that: two clocks started at different moments would disagree
//! about the same deadline by the difference in their epochs. The two seams
//! answer different questions, so they are separate traits.

use std::{sync::Arc, time::Instant};

#[cfg(test)]
use std::time::Duration;

/// Source of monotonic time for the shutdown state machine.
///
/// Deliberately narrow: it reports the current instant and nothing else, so
/// the only thing a test has to control is what the clock says now.
pub(crate) trait MonotonicClock: Send + Sync {
    /// The current point on the monotonic clock.
    fn now(&self) -> Instant;
}

/// [`MonotonicClock`] backed by [`Instant::now`].
///
/// The production implementation. It holds no state and adds no indirection
/// worth measuring: the trait call is one inlined `Instant::now`.
#[derive(Debug, Default)]
pub(crate) struct SystemClock;

impl MonotonicClock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A clock that only moves when a test tells it to.
///
/// Deadlines in this codebase are expressed as `Instant`s, so a manual clock
/// needs an origin to add its offset to. The origin is captured once at
/// construction and never changes; [`advance`](ManualClock::advance) moves the
/// offset, not the origin, which keeps every `Instant` the clock has already
/// handed out meaningful.
///
/// # Examples
///
/// ```ignore
/// let clock = Arc::new(ManualClock::new());
/// let signal = ShutdownSignal::with_clock(clock.clone());
/// signal.begin_shutdown(clock.base() + Duration::from_secs(1));
/// assert!(!signal.is_aborted());
/// clock.advance(Duration::from_secs(2));
/// assert!(signal.is_aborted());
/// ```
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct ManualClock {
    /// The instant all reported times are offset from.
    base: Instant,
    /// How far the clock has been moved past `base`, in an atomic so the
    /// clock stays `Sync` when a test shares it with a worker thread.
    offset_nanos: std::sync::atomic::AtomicU64,
}

#[cfg(test)]
impl ManualClock {
    /// Create a clock stopped at the current instant.
    pub(crate) fn new() -> Self {
        Self {
            base: Instant::now(),
            offset_nanos: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// The instant this clock reported when it was created.
    ///
    /// Deadlines are built from this rather than from a fresh `Instant::now`,
    /// so a test's deadline is expressed against the same origin the queries
    /// are answered from.
    pub(crate) fn base(&self) -> Instant {
        self.base
    }

    /// Move the clock forward by `delta`.
    pub(crate) fn advance(&self, delta: Duration) {
        let delta_nanos = u64::try_from(delta.as_nanos()).unwrap_or(u64::MAX);
        self.offset_nanos
            .fetch_add(delta_nanos, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
impl MonotonicClock for ManualClock {
    fn now(&self) -> Instant {
        let offset = self.offset_nanos.load(std::sync::atomic::Ordering::Relaxed);
        self.base + Duration::from_nanos(offset)
    }
}

/// The clock a [`ShutdownSignal`] was built with.
///
/// A named alias rather than a bare `Arc<dyn ...>` so the field and the
/// constructor signature read as "the signal's clock" instead of as a
/// trait-object incantation repeated at each site.
///
/// [`ShutdownSignal`]: crate::shutdown_signal::ShutdownSignal
pub(crate) type SharedClock = Arc<dyn MonotonicClock>;

/// The production clock, shared by every signal that was not given one.
pub(crate) fn system_clock() -> SharedClock {
    Arc::new(SystemClock)
}
