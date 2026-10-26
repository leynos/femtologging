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

use parking_lot::Mutex;

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
/// A short per-instance mutex serializes the counter recheck, interval claim and
/// drain against [`flush`]. [`record_drop`] remains an atomic increment and does
/// not take that mutex. The mutex is released before either callback runs, so
/// callbacks are **not** serialized against one another and may re-enter the
/// warner. At most one [`warn_if_due`] caller claims an interval: the claim uses
/// compare-exchange, and a caller that loses it returns without draining.
/// Concurrent callbacks therefore receive disjoint batches. A callback that
/// panics abandons the batch it was handed; the counter is not restored, which
/// keeps the "handed off once" guarantee rather than trading it for a
/// possibly-repeated callback.
///
/// The atomics ordering is [`Ordering::Relaxed`] throughout, which is sound
/// because the counter and the timestamp publish only their own values and
/// carry no data other than the count.
///
/// [`record_drop`]: RateLimitedWarner::record_drop
/// [`warn_if_due`]: RateLimitedWarner::warn_if_due
/// [`flush`]: RateLimitedWarner::flush
pub struct RateLimitedWarner {
    /// Serializes due-window claims and counter drains against explicit flushes.
    drain_lock: Mutex<()>,
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
            drain_lock: Mutex::new(()),
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
    /// The counter is checked before sampling the clock and rechecked under a
    /// short mutex before the interval claim and drain. This serializes the due
    /// claim and drain against [`flush`], so a flush that drains the last pending
    /// batch cannot leave an empty interval claimed. At most one caller emits
    /// per interval; callers that lose the compare-exchange return without
    /// draining. The mutex is released before the callback runs, so a slow or
    /// re-entrant callback does not hold up another caller or a flush.
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
        // Avoid clock reads for the common empty case. Recheck under the mutex
        // because a concurrent flush may drain this count before the claim.
        if self.dropped.load(Ordering::Relaxed) == 0 {
            return;
        }
        let now = self.clock.now_millis();
        let count = {
            let _drain_guard = self.drain_lock.lock();
            // A concurrent flush may have taken the observed batch while the
            // clock was sampled. Do not claim an interval when there is nothing
            // left to report.
            if self.dropped.load(Ordering::Relaxed) == 0 {
                return;
            }
            let prev = self.last_warn.load(Ordering::Relaxed);
            if prev != u64::MAX && now.saturating_sub(prev) < self.interval_ms {
                return;
            }
            // `compare_exchange` ensures callers using the same observed
            // timestamp cannot both claim this warning interval.
            if !try_claim_interval(&self.last_warn, prev, now) {
                return;
            }
            // The mutex covers the recheck, claim and swap, so `flush` cannot
            // empty a due batch after its interval has been claimed.
            self.dropped.swap(0, Ordering::Relaxed)
        };
        if count > 0 {
            warn(count);
        }
    }

    /// Immediately warn about any dropped records.
    ///
    /// This is not rate limited: it reports whatever is pending, even inside a
    /// suppression window, and does not consume or restart the interval. Use it
    /// on flush and shutdown paths, where the caller needs the count now rather
    /// than at the next due moment.
    ///
    /// The counter drain is serialized against the due check and interval
    /// claim. The mutex is released before the callback runs.
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
        let count = {
            let _drain_guard = self.drain_lock.lock();
            self.dropped.swap(0, Ordering::Relaxed)
        };
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

/// Claim a warning interval only if `last_warn` still contains the timestamp
/// observed by this caller.
fn try_claim_interval(last_warn: &AtomicU64, prev: u64, now: u64) -> bool {
    last_warn
        .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
}

impl Default for RateLimitedWarner {
    fn default() -> Self {
        Self::new(DEFAULT_WARN_INTERVAL)
    }
}

#[cfg(test)]
#[path = "rate_limited_warner_tests.rs"]
mod tests;

#[cfg(test)]
mod concurrency_tests {
    //! Tests for atomic interval claims and synchronized warning drains.

    use super::{Clock, RateLimitedWarner};
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    /// A clock that pauses the first warning caller inside its time sample.
    struct PausingClock {
        /// Timestamp returned after the test releases the warning caller.
        now_ms: u64,
        /// Pauses only the first clock sample.
        should_pause: AtomicBool,
        /// Synchronizes the warning caller with the test before it pauses.
        entered: Barrier,
        /// Prevents the warning caller from sampling time until released.
        resume: Barrier,
    }

    impl PausingClock {
        /// Create the two-party barriers used by the race test.
        fn new(now_ms: u64) -> Self {
            Self {
                now_ms,
                should_pause: AtomicBool::new(true),
                entered: Barrier::new(2),
                resume: Barrier::new(2),
            }
        }
    }

    impl Clock for PausingClock {
        fn now_millis(&self) -> u64 {
            if self.should_pause.swap(false, Ordering::Relaxed) {
                self.entered.wait();
                self.resume.wait();
            }
            self.now_ms
        }
    }

    /// A flush winning the race does not leave an empty interval claimed.
    #[test]
    fn flush_draining_a_due_call_does_not_claim_its_interval() {
        let clock = Arc::new(PausingClock::new(1000));
        let warner = Arc::new(RateLimitedWarner::with_clock(
            Duration::from_secs(1),
            Arc::clone(&clock) as Arc<dyn Clock>,
        ));
        warner.record_drop();

        let warning_counts = Arc::new(Mutex::new(Vec::new()));
        thread::scope(|scope| {
            let warning_warner = Arc::clone(&warner);
            let warning_counts = Arc::clone(&warning_counts);
            scope.spawn(move || {
                warning_warner.warn_if_due(|count| {
                    warning_counts
                        .lock()
                        .expect("warning counts lock")
                        .push(count);
                });
            });
            clock.entered.wait();

            let mut flush_counts = Vec::new();
            warner.flush(|count| flush_counts.push(count));
            assert_eq!(flush_counts, vec![1]);
            assert_eq!(warner.pending_drops(), 0);
            clock.resume.wait();
        });

        assert!(
            warning_counts
                .lock()
                .expect("warning counts lock")
                .is_empty()
        );
        warner.record_drop();
        warner.warn_if_due(|count| {
            warning_counts
                .lock()
                .expect("warning counts lock")
                .push(count);
        });
        assert_eq!(
            *warning_counts.lock().expect("warning counts lock"),
            vec![1],
            "the flush must not leave the next drop suppressed"
        );
    }
}
