//! Rate-limited warning system for dropped log records.
//!
//! This module provides [`RateLimitedWarner`], which tracks dropped log records
//! and emits warnings at configurable intervals to avoid spamming logs while
//! still alerting users to potential issues.
//!
//! # What this module does not do
//!
//! The warner decides *which* drops are due for a warning and *when* the next
//! warning may be emitted. It does not carry them anywhere: the flush transport
//! that caps a handler's deadline and puts the count on a marker lives in
//! [`flush_handler_until`](crate::handler::flush_handler_until). A handler asks
//! for the pending count with [`RateLimitedWarner::take_pending`] and hands it
//! back with [`RateLimitedWarner::restore_drops`] if the marker never entered
//! its queue. Ordinary warnings keep the interval: they ask
//! [`RateLimitedWarner::take_due`], which declines while the interval runs.
//! Retry bookkeeping stays here because a restored count is not merely pending
//! again: reporting it restarts the warning interval, which is rate-limit
//! policy.
//!
//! # A clock of its own
//!
//! [`Clock`] reports milliseconds since the warner was constructed — an opaque
//! counter for deciding whether an interval elapsed. It is deliberately not
//! [`MonotonicClock`](crate::monotonic_clock::MonotonicClock), which reports an
//! absolute [`Instant`]: a deadline must be compared against another absolute
//! instant, and a counter whose epoch belongs to one object cannot express
//! that. The two seams answer different questions, so they stay separate; see
//! the [`monotonic_clock`](crate::monotonic_clock) module for the same
//! reasoning from that side.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Source of time for deciding when the next dropped-record warning is due.
pub trait Clock: Send + Sync {
    /// Return the current time in milliseconds since an arbitrary epoch.
    fn now_millis(&self) -> u64;
}

/// [`Clock`] implementation backed by [`Instant`], used outside tests.
///
/// Reading the real clock here is the production behaviour, not a test seam
/// leaking through: a warner that never sampled `Instant::now` could not tell
/// whether an interval had elapsed. The epoch is captured at construction, so
/// both the reading and the comparison come from the same monotonic source.
pub struct SystemClock {
    /// Instant captured at construction; elapsed time is reported relative to it.
    start: Instant,
}

impl SystemClock {
    /// Start a clock whose epoch is the instant of construction.
    ///
    /// Explicit rather than hidden behind [`Default`], so a caller that needs
    /// a specific epoch — a test double, or a warner restored alongside its
    /// handler — can say so.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
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
        Self::with_clock(interval, Arc::new(SystemClock::default()))
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

    /// Number of drops awaiting the next warning emission.
    ///
    /// Exists so a test can prove that a path which deliberately does not
    /// count a loss leaves this figure untouched, rather than inferring it
    /// from the absence of a warning.
    #[cfg(test)]
    pub(crate) fn pending_drops(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
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
    /// This is the ordinary warning path, so the interval decides whether a
    /// count may be taken. An explicit flush asks for the pending figure
    /// regardless of the interval, and uses [`Self::take_pending`] instead.
    ///
    /// The number is destructive: the count leaves the warner here, so a caller
    /// that cannot deliver it must give it back with [`Self::restore_drops`].
    /// Flush handlers hand it to
    /// [`flush_handler_until`](crate::handler::flush_handler_until), which puts
    /// it on the marker for the existing worker; warning there rather than here
    /// keeps a synchronous `log` callback from holding the flush caller beyond
    /// its deadline.
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

    /// Hand back a count that [`Self::take_due`] or [`Self::take_pending`]
    /// removed but could not deliver.
    ///
    /// A restored count is reported by the next [`Self::take_due`] or
    /// [`Self::flush`] even inside the warning interval, because the interval
    /// only limits how often warnings are emitted, not whether records are
    /// accounted for.
    pub(crate) fn restore_drops(&self, count: u64) {
        self.retry_drops.fetch_add(count, Ordering::Relaxed);
    }

    /// Take every pending warning count, ignoring the interval.
    ///
    /// An explicit flush asks what has been lost since the last report, so the
    /// interval must not withhold the figure: a drop recorded moments after a
    /// warning is still one the caller is owed, and a flush that returned
    /// success without it would undercount. Reporting the count restarts the
    /// interval, exactly as [`Self::flush`] does.
    ///
    /// The number is destructive for the same reason as [`Self::take_due`], so
    /// a caller that cannot deliver it must restore it with
    /// [`Self::restore_drops`].
    pub(crate) fn take_pending(&self) -> Option<u64> {
        let retry_count = self.retry_drops.swap(0, Ordering::Relaxed);
        let count = self
            .dropped
            .swap(0, Ordering::Relaxed)
            .saturating_add(retry_count);
        if count == 0 {
            return None;
        }
        self.last_warn
            .store(self.clock.now_millis(), Ordering::Relaxed);
        Some(count)
    }

    /// Immediately warn about pending drops and restart the rate limit interval.
    ///
    /// A count held back because an earlier flush could not enter the worker
    /// queue is reported here too, so a restored count is never stranded.
    pub fn flush(&self, mut warn: impl FnMut(u64)) {
        if let Some(count) = self.take_pending() {
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
