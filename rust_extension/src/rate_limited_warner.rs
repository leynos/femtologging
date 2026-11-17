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
//!
//! The warner takes no lock. Draining is one atomic swap and claiming an
//! interval is one compare-exchange, so a `fork` copies plain counters and can
//! never strand a child on a lock word some parent thread held at the moment of
//! the fork. A child does inherit the pending count, so it may report a
//! pre-fork batch the parent also reports; that is two reports of one count,
//! not two drains of one counter.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Source of time for [`RateLimitedWarner`] and the stream handler's flush deadline.
pub trait Clock: Send + Sync {
    /// Return the current time in milliseconds since an arbitrary epoch.
    fn now_millis(&self) -> u64;
}

/// [`Clock`] implementation backed by [`Instant`], used outside tests.
pub struct SystemClock {
    /// Instant captured at construction; elapsed time is reported relative to it.
    start: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
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
/// A due call takes its batch with an atomic swap and only then claims the
/// interval, restoring the batch if the claim is refused. Draining first means
/// a concurrent [`flush`] cannot empty a batch this call has already claimed an
/// interval for, so a flush never leaves the next warning suppressed for a
/// whole interval; restoring on a refused claim means a lost race costs the
/// caller nothing but a retry on its next due moment. At most one
/// [`warn_if_due`] caller claims an interval: the claim uses compare-exchange,
/// and a caller that loses it returns without reporting. Concurrent callbacks
/// therefore receive disjoint batches.
///
/// Neither the clock nor the callbacks run under any lock, because the warner
/// takes none. The clock is a caller-supplied seam, so a clock that re-enters
/// the warner is safe, and callbacks are **not** serialized against one another
/// and may re-enter the warner.
///
/// A drained batch is restored if the report is abandoned before the callback
/// runs, including when a clock sample panics. A callback that panics has
/// already been handed the count, so it abandons it: restoring there could
/// report the same count twice.
///
/// An interval starts when a report is *published*, not when its caller first
/// read the clock. The claim publishes its sample, then corrects it forward if
/// the clock has moved on by the time the claim was won. That correction is
/// also the ownership check: if a caller delayed between its claim and its
/// publication sample lets another caller publish a later interval, the first
/// caller's compare-exchange fails and it does not report. Otherwise both would
/// warn back to back, each nominally inside its own interval.
///
/// The counter and timestamp use [`Ordering::Relaxed`], which is sound because
/// they publish only their own values and carry no data other than the count.
/// Nothing is held across a thread boundary, so `fork` copies only these plain
/// counters.
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
        Self::with_clock(interval, Arc::new(SystemClock::default()))
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
    /// The callback receives the number of drops accumulated since the last
    /// report, which includes those recorded while earlier calls were
    /// suppressed. An empty call does not start an interval, so a poller with
    /// nothing to report cannot delay the next real warning. A call that
    /// arrives inside the interval leaves the pending count untouched for the
    /// next due call or for [`flush`].
    ///
    /// The counter is checked before sampling the clock and rechecked by the
    /// claim itself. A call that takes a batch and then loses the interval
    /// restores the batch, so the drops stay pending for the next due call. The
    /// same applies to a call that wins a claim but is then delayed until
    /// another caller has published a later interval: it is superseded, and
    /// restoring rather than reporting avoids two warnings back to back. At
    /// most one caller emits per interval, and nothing is held while the
    /// callback runs, so a slow or re-entrant callback does not hold up another
    /// caller or a flush, and a re-entrant clock cannot wait for the claim it
    /// is timing.
    ///
    /// A call whose interval is still open returns after one clock read and one
    /// atomic load, without touching the counter. The refusal paths call this
    /// method for every dropped record, so under sustained overload the
    /// suppressed calls are the common case and must stay cheap. The pre-check
    /// reads `last_warn` without any synchronization, which is sound because
    /// the timestamp only moves forward: a stale read can only *over*-report
    /// that a warning is due, and the claim's own recheck corrects that; it can
    /// never hide a due warning.
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
        if !self.is_warning_due() {
            return;
        }
        if let Some(pending) = self.claim_due_batch() {
            warn(pending.take());
        }
    }

    /// Report whether a warning could be due, without touching the counter.
    ///
    /// Avoids clock reads for the common empty case, then compares the stale
    /// `last_warn` read against the interval. The read is unsynchronized
    /// because the timestamp only moves forward: a stale read can only claim a
    /// warning is due when it is not, and [`Self::claim_due_batch`] decides
    /// that again before it publishes anything.
    fn is_warning_due(&self) -> bool {
        if self.dropped.load(Ordering::Relaxed) == 0 {
            return false;
        }
        let now = self.clock.now_millis();
        let observed = self.last_warn.load(Ordering::Relaxed);
        observed == u64::MAX || now.saturating_sub(observed) >= self.interval_ms
    }

    /// Take the pending batch and claim the interval covering it, or return
    /// `None` with the batch left pending.
    ///
    /// The swap happens before the claim so that a concurrent [`Self::flush`]
    /// can never empty a batch whose interval this call has already claimed:
    /// that would suppress the next warning for a whole interval while nothing
    /// was reported for it. A flush that wins the swap race instead takes the
    /// batch and this call finds nothing to report, so it claims nothing.
    ///
    /// The returned guard owns the batch. It puts the batch back if either
    /// clock sample panics, and it is discharged only once the callback is
    /// invoked, so an abandoned report cannot silently lose drops.
    fn claim_due_batch(&self) -> Option<PendingBatch<'_>> {
        let batch = self.dropped.swap(0, Ordering::Relaxed);
        if batch == 0 {
            return None;
        }
        // From here the batch is out of the counter, so the guard restores it
        // on every early return and on unwind.
        let pending = PendingBatch {
            warner: self,
            count: batch,
        };
        let claimed_at = self.claim_interval()?;
        if !self.publish_report_time(claimed_at) {
            // A later interval was published while this caller was delayed, so
            // that report supersedes this one; the guard restores the batch
            // for the next due call instead of emitting two warnings together.
            return None;
        }
        Some(pending)
    }

    /// Claim the warning interval, returning the timestamp published, if any.
    ///
    /// The clock is sampled immediately before the compare-exchange, with no
    /// lock or wait in between. A refused claim samples the clock but touches
    /// neither counter.
    fn claim_interval(&self) -> Option<u64> {
        let now = self.clock.now_millis();
        let prev = self.last_warn.load(Ordering::Relaxed);
        if prev != u64::MAX && now.saturating_sub(prev) < self.interval_ms {
            return None;
        }
        // `compare_exchange` ensures callers that observed the same timestamp
        // cannot both claim this warning interval.
        try_claim_interval(&self.last_warn, prev, now).then_some(now)
    }

    /// Publish the report time, reporting whether this caller still owns the
    /// interval it claimed.
    ///
    /// The claim's sample was taken before the compare-exchange, so this sample
    /// moves the published start forward to the moment of publication; a clock
    /// that steps backwards cannot shorten an interval because the value never
    /// moves back.
    ///
    /// The same exchange is the liveness check for the claim. It fails when
    /// another caller has already published a later interval - possible only
    /// while this caller was delayed between its claim and this sample - and
    /// that caller's report supersedes this one. Emitting here would place two
    /// warnings back to back even though each is inside its own interval, so a
    /// false result means the caller must not report.
    fn publish_report_time(&self, claimed_at: u64) -> bool {
        let reported_at = self.clock.now_millis().max(claimed_at);
        self.last_warn
            .compare_exchange(
                claimed_at,
                reported_at,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
    }

    /// Immediately warn about any dropped records.
    ///
    /// This is not rate limited: it reports whatever is pending, even inside a
    /// suppression window, and does not consume or restart the interval. Use it
    /// on flush and shutdown paths, where the caller needs the count now rather
    /// than at the next due moment.
    ///
    /// The drain is one atomic swap, so it cannot interleave with a due call's
    /// batch and no lock is held while the callback runs.
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

/// Claim a warning interval only if `last_warn` still contains the timestamp
/// observed by this caller.
fn try_claim_interval(last_warn: &AtomicU64, prev: u64, now: u64) -> bool {
    last_warn
        .compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
}

/// A drained batch that is put back unless it is handed to a callback.
///
/// A due call removes the batch from the counter before it samples the clock
/// and claims the interval, so every path that gives up between those steps
/// must restore it. Owning the count in a guard makes that the default: the
/// early returns and any unwind both put the count back.
///
/// [`Self::take`] disarms the guard, so the count is restored only when the
/// report was abandoned. It is called immediately before invoking the callback,
/// which keeps the "handed off once" guarantee: a callback that panics has been
/// given the count, so re-adding it could report the same count twice.
struct PendingBatch<'a> {
    /// Warner whose counter the batch came from and may return to.
    warner: &'a RateLimitedWarner,
    /// Count still owned by this guard; zero once it is handed off.
    count: u64,
}

impl PendingBatch<'_> {
    /// Disarm the guard and return the count it owned.
    fn take(mut self) -> u64 {
        let count = self.count;
        self.count = 0;
        count
    }
}

impl Drop for PendingBatch<'_> {
    fn drop(&mut self) {
        if self.count > 0 {
            self.warner.dropped.fetch_add(self.count, Ordering::Relaxed);
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

#[cfg(test)]
#[path = "rate_limited_warner_claim_tests.rs"]
mod claim_tests;

#[cfg(test)]
#[path = "rate_limited_warner_panic_tests.rs"]
mod panic_tests;

#[cfg(test)]
#[path = "rate_limited_warner_concurrency_tests.rs"]
mod concurrency_tests;
