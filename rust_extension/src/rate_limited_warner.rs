//! Rate-limited warning system for dropped log records.
//!
//! This module provides [`RateLimitedWarner`], which tracks dropped log records
//! and emits warnings at configurable intervals to avoid spamming logs while
//! still alerting users to potential issues.
//!
//! The counter is conserved: a warning attempt that falls inside the interval
//! does **not** discard the drops it found. Those drops stay pending and are
//! included in the next warning, so the counts an operator reads account for
//! every lost record. An explicit [`RateLimitedWarner::flush`] reports the
//! pending total immediately, whatever the clock says, which is what makes it
//! safe to rely on during shutdown.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

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
/// [`warn_if_due`] emits a warning using the provided callback if the
/// configured interval has elapsed. [`flush`] emits a warning immediately if
/// any records have been dropped since the last emission.
///
/// # Concurrency
///
/// Each increment is either removed by exactly one call to [`warn_if_due`] or
/// [`flush`] and handed to its callback, or remains pending in the counter until a
/// later drain. No drain can claim the same increment twice, and a suppressed call
/// does not discard it. A drop recorded while another thread is warning belongs to
/// that batch or to the next one, not to both.
///
/// The types are safe to share across threads, and the callbacks are **not**
/// serialized against one another. At most one [`warn_if_due`] caller emits per
/// interval: the interval is claimed with a compare-exchange, so a loser of
/// that race returns without draining. Concurrent callbacks therefore observe
/// disjoint batches. A callback that panics abandons the batch it was handed;
/// the counter is not restored, which keeps the "reported once" guarantee
/// rather than trading it for a possibly-repeated report.
///
/// The atomics ordering is [`Ordering::Relaxed`] throughout, which is sound
/// because the counter and the timestamp publish only their own values and
/// carry no data other than the count.
///
/// [`record_drop`]: RateLimitedWarner::record_drop
/// [`warn_if_due`]: RateLimitedWarner::warn_if_due
/// [`flush`]: RateLimitedWarner::flush
pub struct RateLimitedWarner {
    /// Millisecond timestamp of the last warning, updated atomically by emitters.
    last_warn: AtomicU64,
    /// Number of dropped records waiting for the next warning emission.
    dropped: AtomicU64,
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
    /// The callback receives the number of drops accumulated since the last
    /// report, which includes those recorded while earlier calls were
    /// suppressed. An empty call does not start an interval, so a poller with
    /// nothing to report cannot delay the next real warning. A call that
    /// arrives inside the interval leaves the pending count untouched for the
    /// next due call or for [`flush`].
    ///
    /// At most one caller emits per interval; callers that lose the race to
    /// claim it return without reporting. The callback runs after the interval
    /// has been claimed, so a slow callback does not hold the interval open for
    /// another thread to re-claim, and a re-entrant callback sees the current
    /// interval as already started.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use _femtologging_rs::rate_limited_warner::RateLimitedWarner;
    ///
    /// let warner = RateLimitedWarner::new(Duration::from_secs(5));
    /// warner.record_drop();
    /// let mut reported = Vec::new();
    /// warner.warn_if_due(|count| reported.push(count));
    /// assert_eq!(reported, vec![1]);
    /// ```
    ///
    /// [`flush`]: RateLimitedWarner::flush
    pub fn warn_if_due(&self, mut warn: impl FnMut(u64)) {
        // Nothing to say: report nothing, and leave the interval unclaimed so
        // that the first real drop is not held back by a poller.
        if self.dropped.load(Ordering::Relaxed) == 0 {
            return;
        }
        let now = self.clock.now_millis();
        let prev = self.last_warn.load(Ordering::Relaxed);
        if prev != u64::MAX && now.saturating_sub(prev) < self.interval_ms {
            return;
        }
        // Claim the interval before draining, so only one caller reports it and
        // no two callbacks are handed the same batch. `compare_exchange` rather
        // than a load-then-store: two callers can both see the same expired
        // `prev`, and a plain store would let both report and let the later one
        // move the timestamp backwards. The sentinel's replacement is the one
        // downward write; every write after it is strictly increasing, being at
        // least one interval past the value it replaced.
        if self
            .last_warn
            .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        self.report_pending(&mut warn);
    }

    /// Immediately warn about any dropped records.
    ///
    /// This is not rate limited: it reports whatever is pending, even inside a
    /// suppression window, and does not consume or restart the interval. Use it
    /// on flush and shutdown paths, where the caller needs the count now rather
    /// than at the next due moment.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use _femtologging_rs::rate_limited_warner::RateLimitedWarner;
    ///
    /// let warner = RateLimitedWarner::new(Duration::from_secs(3600));
    /// warner.record_drop();
    /// warner.record_drop();
    /// let mut reported = Vec::new();
    /// warner.flush(|count| reported.push(count));
    /// assert_eq!(reported, vec![2]);
    /// ```
    pub fn flush(&self, mut warn: impl FnMut(u64)) {
        self.report_pending(&mut warn);
    }

    /// Drain the counter and hand a non-zero batch to `warn`.
    ///
    /// The `swap` is what makes the batch exclusive: the increments it returns
    /// are removed from the counter, so no other drain can see them, and a
    /// concurrent `record_drop` lands either in this batch or in the next one.
    fn report_pending(&self, warn: &mut impl FnMut(u64)) {
        let count = self.dropped.swap(0, Ordering::Relaxed);
        if count > 0 {
            warn(count);
        }
    }

    /// Return the number of drops recorded but not yet reported.
    ///
    /// Exposed for tests that must assert conservation directly; the value is
    /// read with [`Ordering::Relaxed`] and is a snapshot, not a reservation.
    ///
    /// Gated on `test` as well as `test-util`: this module's own unit tests run
    /// in every feature lane, including the ones that do not enable `test-util`.
    #[cfg(any(test, feature = "test-util"))]
    pub fn pending_drops(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
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
