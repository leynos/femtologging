//! Bounded, process-wide metrics and diagnostics for logger flushes.
//!
//! Every completed flush is classified by its terminal outcome and timed into
//! a fixed-bucket histogram. Both live in one process-wide [`FlushMetrics`]
//! instance, so a process has a bounded number of counters however many loggers
//! it creates. A flush is not on the record path, so the relaxed atomics here
//! cost nothing that matters, and no measurement allocates.
//!
//! The metric names exported by this module are:
//!
//! - `femtologging.flush.total{outcome}` with the labels `success`,
//!   `ack_timeout`, `channel_closed` and `handler_failure`;
//! - `femtologging.flush.duration_us`, a cumulative histogram whose finite
//!   bucket bounds are [`DURATION_BUCKET_BOUNDS_US`] plus an overflow bucket;
//! - `femtologging.flush.bridge_failures`, the number of logger flushes that
//!   failed while the `log` bridge flushed every registered logger.
//!
//! Diagnostics never carry record payloads or logger names: both are unbounded
//! user data, and a name per failure would turn a pathological handler into an
//! unbounded log stream. Instead the flush path emits one bounded tracing event
//! per flush (see `FemtoLogger::flush_handlers_blocking`) and rate-limited
//! warnings that report only how many failures accumulated since the previous
//! warning.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::rate_limited_warner::{DEFAULT_WARN_INTERVAL, RateLimitedWarner};

/// Upper bounds of the finite flush-duration histogram buckets, in microseconds.
///
/// Every bound is inclusive. The histogram has one bucket more than this list:
/// the final overflow bucket counts every observation, however long it took.
pub(crate) const DURATION_BUCKET_BOUNDS_US: [u64; 8] =
    [100, 500, 1_000, 5_000, 10_000, 50_000, 100_000, 1_000_000];

/// Number of cumulative duration buckets, including the overflow bucket.
const DURATION_BUCKET_COUNT: usize = DURATION_BUCKET_BOUNDS_US.len() + 1;

/// Number of terminal flush outcomes.
const OUTCOME_COUNT: usize = 4;

/// Terminal result of one logger flush attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlushOutcome {
    /// The shard drained and every attached handler flushed successfully.
    Success,
    /// The worker did not acknowledge the drain barrier within its timeout.
    AckTimeout,
    /// The dispatch channel was closed, so no barrier could be answered.
    ChannelClosed,
    /// The drain succeeded but at least one attached handler reported failure.
    HandlerFailure,
}

impl FlushOutcome {
    /// Return the bounded label used in metrics and diagnostics.
    ///
    /// The label is a fixed string rather than a formatted message so the set
    /// of observable values cannot grow with user input. Only the tracing span
    /// consumes it, so it is compiled out when that feature is off.
    #[cfg(feature = "tracing-compat")]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::AckTimeout => "ack_timeout",
            Self::ChannelClosed => "channel_closed",
            Self::HandlerFailure => "handler_failure",
        }
    }

    /// Return this outcome's index among the outcome counters.
    const fn index(self) -> usize {
        match self {
            Self::Success => 0,
            Self::AckTimeout => 1,
            Self::ChannelClosed => 2,
            Self::HandlerFailure => 3,
        }
    }

    /// Return whether the flush completed successfully.
    pub(crate) const fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }
}

/// Counters and histogram shared by every logger in a process.
struct FlushMetrics {
    /// One counter per terminal outcome, indexed by [`FlushOutcome::index`].
    outcomes: [AtomicU64; OUTCOME_COUNT],
    /// Cumulative counts at or below each finite bound, then the overflow bucket.
    duration_buckets: [AtomicU64; DURATION_BUCKET_COUNT],
    /// Total number of recorded flush durations.
    duration_count: AtomicU64,
    /// Sum of every recorded flush duration, in microseconds.
    duration_sum_us: AtomicU64,
    /// Logger flushes that failed while the `log` bridge flushed every logger.
    bridge_failures: AtomicU64,
    /// Rate limiter for acknowledgement-timeout warnings.
    timeout_warner: RateLimitedWarner,
    /// Rate limiter for handler-failure warnings.
    handler_failure_warner: RateLimitedWarner,
    /// Rate limiter for log-bridge flush-failure warnings.
    bridge_failure_warner: RateLimitedWarner,
}

impl FlushMetrics {
    /// Create counters at zero and warners at their default interval.
    fn new() -> Self {
        Self {
            outcomes: std::array::from_fn(|_| AtomicU64::new(0)),
            duration_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            duration_count: AtomicU64::new(0),
            duration_sum_us: AtomicU64::new(0),
            bridge_failures: AtomicU64::new(0),
            timeout_warner: RateLimitedWarner::new(DEFAULT_WARN_INTERVAL),
            handler_failure_warner: RateLimitedWarner::new(DEFAULT_WARN_INTERVAL),
            bridge_failure_warner: RateLimitedWarner::new(DEFAULT_WARN_INTERVAL),
        }
    }

    /// Record one completed flush and emit a warning when it failed.
    fn record(&self, outcome: FlushOutcome, elapsed: Duration) {
        let micros = elapsed_micros(elapsed);
        self.outcomes[outcome.index()].fetch_add(1, Ordering::Relaxed);
        for (index, bound) in DURATION_BUCKET_BOUNDS_US.iter().enumerate() {
            if micros <= *bound {
                self.duration_buckets[index].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.duration_buckets[DURATION_BUCKET_BOUNDS_US.len()].fetch_add(1, Ordering::Relaxed);
        self.duration_count.fetch_add(1, Ordering::Relaxed);
        self.duration_sum_us.fetch_add(micros, Ordering::Relaxed);
        self.warn_at_boundary(outcome);
    }

    /// Record a logger flush that failed during a `log`-bridge flush sweep.
    fn record_bridge_failure(&self) {
        self.bridge_failures.fetch_add(1, Ordering::Relaxed);
        self.bridge_failure_warner.record_drop();
        self.bridge_failure_warner.warn_if_due(|count| {
            log::warn!("femtologging: {count} logger flush(es) failed during a log-bridge flush");
        });
    }

    /// Emit the rate-limited warning appropriate to a failed outcome.
    fn warn_at_boundary(&self, outcome: FlushOutcome) {
        match outcome {
            FlushOutcome::AckTimeout => self.warn_ack_timeouts(),
            FlushOutcome::HandlerFailure => self.warn_handler_failures(),
            FlushOutcome::Success | FlushOutcome::ChannelClosed => {}
        }
    }

    /// Rate-limit warnings about drain barriers that timed out.
    fn warn_ack_timeouts(&self) {
        self.timeout_warner.record_drop();
        self.timeout_warner.warn_if_due(|count| {
            log::warn!(
                "femtologging: {count} flush acknowledgement timeout(s) since the last warning"
            );
        });
    }

    /// Rate-limit warnings about handlers whose flush reported failure.
    fn warn_handler_failures(&self) {
        self.handler_failure_warner.record_drop();
        self.handler_failure_warner.warn_if_due(|count| {
            log::warn!("femtologging: {count} handler flush failure(s) since the last warning");
        });
    }

    /// Read every counter and bucket into a consistent-enough snapshot.
    #[cfg(test)]
    fn snapshot(&self) -> FlushMetricsSnapshot {
        FlushMetricsSnapshot {
            outcomes: std::array::from_fn(|index| self.outcomes[index].load(Ordering::Relaxed)),
            duration_buckets: std::array::from_fn(|index| {
                self.duration_buckets[index].load(Ordering::Relaxed)
            }),
            duration_count: self.duration_count.load(Ordering::Relaxed),
            duration_sum_us: self.duration_sum_us.load(Ordering::Relaxed),
            bridge_failures: self.bridge_failures.load(Ordering::Relaxed),
        }
    }
}

/// A point-in-time copy of the process-wide flush metrics.
#[cfg(test)]
pub(crate) struct FlushMetricsSnapshot {
    /// One counter per terminal outcome, indexed by [`FlushOutcome::index`].
    pub(crate) outcomes: [u64; OUTCOME_COUNT],
    /// Cumulative counts at or below each finite bound, then the overflow bucket.
    pub(crate) duration_buckets: [u64; DURATION_BUCKET_COUNT],
    /// Total number of recorded flush durations.
    pub(crate) duration_count: u64,
    /// Sum of every recorded flush duration, in microseconds.
    pub(crate) duration_sum_us: u64,
    /// Logger flushes that failed while the `log` bridge flushed every logger.
    pub(crate) bridge_failures: u64,
}

/// Convert a duration to whole microseconds, saturating rather than wrapping.
///
/// Shared with the tracing span so the recorded field and the histogram agree
/// on units and on what an overflowing duration reports.
pub(crate) fn elapsed_micros(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

/// Return the process-wide flush metrics, creating them on first use.
fn metrics() -> &'static FlushMetrics {
    static METRICS: OnceLock<FlushMetrics> = OnceLock::new();
    METRICS.get_or_init(FlushMetrics::new)
}

/// Record one completed logger flush.
pub(crate) fn record_flush(outcome: FlushOutcome, elapsed: Duration) {
    metrics().record(outcome, elapsed);
}

/// Record a logger flush that failed during a `log`-bridge flush sweep.
pub(crate) fn record_bridge_flush_failure() {
    metrics().record_bridge_failure();
}

#[cfg(test)]
mod tests {
    //! Increment checks for the flush counters and duration histogram.

    use super::*;

    /// A success lands in its own counter and the matching duration bucket.
    #[test]
    fn a_success_increments_its_counter_and_bucket() {
        let metrics = FlushMetrics::new();
        metrics.record(FlushOutcome::Success, Duration::from_micros(250));

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.outcomes, [1, 0, 0, 0]);
        assert_eq!(snapshot.duration_count, 1);
        assert_eq!(snapshot.duration_sum_us, 250);
        // 250 us is above the 100 us bound and at or below 500 us, so it lands
        // in the second finite bucket and every bucket after it.
        assert_eq!(snapshot.duration_buckets, [0, 1, 1, 1, 1, 1, 1, 1, 1]);
    }

    /// A duration longer than every finite bound lands only in the overflow bucket.
    #[test]
    fn a_long_duration_lands_only_in_the_overflow_bucket() {
        let metrics = FlushMetrics::new();
        metrics.record(FlushOutcome::AckTimeout, Duration::from_secs(4));

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.outcomes, [0, 1, 0, 0]);
        assert_eq!(snapshot.duration_buckets, [0, 0, 0, 0, 0, 0, 0, 0, 1]);
    }

    /// Every outcome has a distinct bounded label and its own counter.
    #[test]
    fn each_outcome_is_counted_separately() {
        let metrics = FlushMetrics::new();
        for outcome in [
            FlushOutcome::Success,
            FlushOutcome::AckTimeout,
            FlushOutcome::ChannelClosed,
            FlushOutcome::HandlerFailure,
        ] {
            metrics.record(outcome, Duration::ZERO);
        }

        assert_eq!(metrics.snapshot().outcomes, [1, 1, 1, 1]);
    }

    /// The bridge failure counter rises without touching the per-logger outcomes.
    #[test]
    fn bridge_failures_have_their_own_counter() {
        let metrics = FlushMetrics::new();
        metrics.record_bridge_failure();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.bridge_failures, 1);
        assert_eq!(snapshot.outcomes, [0, 0, 0, 0]);
    }
}
