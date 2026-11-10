//! Rate-limited warning system for dropped log records.
//!
//! This module provides [`RateLimitedWarner`], which tracks dropped log records
//! and emits warnings at configurable intervals to avoid spamming logs while
//! still alerting users to potential issues.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::handler::{QueueFlushFailure, flush_command_until};

/// Source of time for [`RateLimitedWarner`].
pub trait Clock: Send + Sync {
    /// Return the current time in milliseconds since an arbitrary epoch.
    fn now_millis(&self) -> u64;
}

/// [`Clock`] implementation backed by [`Instant`].
struct RealClock {
    /// Instant captured at construction; elapsed time is reported relative to it.
    start: Instant,
}

impl Default for RealClock {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Clock for RealClock {
    fn now_millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

/// How often to emit warnings about dropped log records by default.
pub const DEFAULT_WARN_INTERVAL: Duration = Duration::from_secs(5);

/// Helper that rate limits dropped-record warnings.
///
/// The caller increments the drop counter via [`record_drop`]. The next call to
/// [`warn_if_due`] emits a warning using the provided callback if the configured
/// interval has elapsed. [`flush`] emits a warning immediately if any records
/// have been dropped since the last emission.
pub struct RateLimitedWarner {
    /// Millisecond timestamp of the last warning, updated atomically by emitters.
    last_warn: AtomicU64,
    /// Number of dropped records waiting for the next warning emission.
    dropped: AtomicU64,
    /// Due warning counts whose flush command could not enter a worker queue.
    retry_drops: AtomicU64,
    /// Minimum interval between warning callbacks, stored in milliseconds.
    interval_ms: u64,
    /// Shared clock seam used by production code and deterministic tests.
    clock: Arc<dyn Clock>,
}

impl RateLimitedWarner {
    /// Create a new [`RateLimitedWarner`] using the provided interval.
    pub fn new(interval: Duration) -> Self {
        Self::with_clock(interval, Arc::new(RealClock::default()))
    }

    /// Create a new [`RateLimitedWarner`] with a custom clock.
    pub fn with_clock(interval: Duration, clock: Arc<dyn Clock>) -> Self {
        let interval_ms = interval.as_millis() as u64;
        Self {
            last_warn: AtomicU64::new(u64::MAX),
            dropped: AtomicU64::new(0),
            retry_drops: AtomicU64::new(0),
            interval_ms,
            clock,
        }
    }

    /// Increment the dropped-record counter.
    pub fn record_drop(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// Emit a warning if the rate limit interval has elapsed.
    ///
    /// Records dropped inside the interval are not forgotten: their count stays
    /// pending and is reported with the next warning, so the figure is "records
    /// affected since the last warning" rather than "records in the tick that
    /// happened to warn". Clearing on a suppressed call would undercount
    /// exactly the burst the limiter exists to summarize — a stream of
    /// failures would report one per interval however many it lost. The count
    /// includes the record whose call triggers the warning.
    pub fn warn_if_due(&self, mut warn: impl FnMut(u64)) {
        if let Some(count) = self.take_due() {
            warn(count);
        }
    }

    /// Take a due warning count without invoking its callback.
    ///
    /// Flush callers move the count onto the existing handler worker so a
    /// synchronous `log` callback cannot hold the caller beyond its deadline.
    pub(crate) fn take_due(&self) -> Option<u64> {
        let retry_count = self.retry_drops.swap(0, Ordering::Relaxed);
        if retry_count > 0 {
            self.last_warn
                .store(self.clock.now_millis(), Ordering::Relaxed);
            return Some(retry_count);
        }

        let now = self.clock.now_millis();
        let prev = self.last_warn.load(Ordering::Relaxed);
        if prev == u64::MAX || now.saturating_sub(prev) >= self.interval_ms {
            let count = self.dropped.swap(0, Ordering::Relaxed);
            self.last_warn.store(now, Ordering::Relaxed);
            (count > 0).then_some(count)
        } else {
            // Keep suppressed drops pending so the next warning accounts for
            // the whole interval, including calls that arrived during it.
            None
        }
    }

    /// Retry a due warning when a handler could not enqueue its flush.
    pub(crate) fn restore_drops(&self, count: u64) {
        self.retry_drops.fetch_add(count, Ordering::Relaxed);
    }

    /// Enqueue a flush command that carries the due drop count.
    ///
    /// The count leaves the warner before the send and goes back if the
    /// command never entered the queue. Restoring it keeps the figure attached
    /// to the next warning: a saturated or closed queue would otherwise strand
    /// the drops this flush was meant to report.
    pub(crate) fn flush_with_pending_drops<A, Ack>(
        &self,
        deadline: Instant,
        now: &mut dyn FnMut() -> Instant,
        enqueue: impl FnOnce(Option<u64>, Duration) -> Option<Ack>,
    ) -> Result<A, QueueFlushFailure>
    where
        Ack: FnOnce(Duration) -> Option<A>,
    {
        let pending_drops = self.take_due();
        let outcome = flush_command_until(deadline, now, |budget| enqueue(pending_drops, budget));
        if matches!(outcome, Err(QueueFlushFailure::Enqueue))
            && let Some(count) = pending_drops
        {
            self.restore_drops(count);
        }
        outcome
    }

    /// Immediately warn about pending drops and restart the rate limit interval.
    ///
    /// A count held back because an earlier flush could not enter the worker
    /// queue is reported here too, so a restored count is never stranded.
    pub fn flush(&self, mut warn: impl FnMut(u64)) {
        let retry_count = self.retry_drops.swap(0, Ordering::Relaxed);
        let count = self
            .dropped
            .swap(0, Ordering::Relaxed)
            .saturating_add(retry_count);
        if count > 0 {
            self.last_warn
                .store(self.clock.now_millis(), Ordering::Relaxed);
            warn(count);
        }
    }
}

impl Default for RateLimitedWarner {
    fn default() -> Self {
        Self::new(DEFAULT_WARN_INTERVAL)
    }
}

#[cfg(test)]
#[path = "rate_limited_warner_tests.rs"]
mod tests;
