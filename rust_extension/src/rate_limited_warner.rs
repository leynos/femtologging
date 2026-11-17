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
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
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

/// Mutual exclusion for the drain path that survives `fork`.
///
/// A parking-lot mutex cannot be recovered by a forked child: when the parent
/// forks while a *different* thread holds the lock, the child inherits the
/// locked word with no thread that will ever release it, so the child's first
/// warning would wait forever. Loggers are routinely inherited by
/// `multiprocessing` children, so the drain lock records the owning process
/// instead: only a live thread of *this* process can hold it, and a word
/// holding another process ID is the parent's copy, which the child may clear.
///
/// The critical section is two atomic reads, a compare-exchange and a swap, so
/// waiters spin with a yield rather than parking. Parking would reintroduce the
/// fork problem in a different form: a child cannot wake a parked waiter.
struct ForkSafeDrainLock {
    /// Owning process ID, or zero when the lock is free.
    owner_process: AtomicU32,
}

impl ForkSafeDrainLock {
    /// Create a free lock.
    const fn new() -> Self {
        Self {
            owner_process: AtomicU32::new(0),
        }
    }

    /// Take the lock, waiting for a live thread of this process to release it.
    fn lock(&self) -> DrainGuard<'_> {
        let process_id = std::process::id();
        loop {
            if let Some(guard) = self.try_lock(process_id) {
                return guard;
            }
            std::hint::spin_loop();
            std::thread::yield_now();
        }
    }

    /// Take the lock for `process_id` unless a live thread of that process
    /// holds it. Returns `None` when the caller must retry.
    fn try_lock(&self, process_id: u32) -> Option<DrainGuard<'_>> {
        loop {
            let owner = self.owner_process.load(Ordering::Relaxed);
            if owner == process_id {
                return None;
            }
            if owner != 0 {
                // A different PID cannot be a live thread of this process, so
                // the owner did not survive into it: this is a reservation
                // copied by `fork`. Clear it and try again for the free word;
                // a failed clear is harmless because the next pass re-reads it.
                let _ = self.owner_process.compare_exchange(
                    owner,
                    0,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                );
                continue;
            }
            return self
                .owner_process
                .compare_exchange(0, process_id, Ordering::Acquire, Ordering::Relaxed)
                .ok()
                .map(|_| DrainGuard {
                    lock: self,
                    process_id,
                });
        }
    }
}

/// Exclusive access to a [`ForkSafeDrainLock`], released on drop.
struct DrainGuard<'a> {
    /// Lock this guard took.
    lock: &'a ForkSafeDrainLock,
    /// Process ID this guard wrote into the lock.
    process_id: u32,
}

impl Drop for DrainGuard<'_> {
    fn drop(&mut self) {
        // Release only a word this guard still owns. A forked child that
        // reclaimed an inherited guard has a different owner by now, and this
        // guard must not free the lock out from under it.
        let _ = self.lock.owner_process.compare_exchange(
            self.process_id,
            0,
            Ordering::Release,
            Ordering::Relaxed,
        );
    }
}

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
/// A short per-instance lock serializes the counter recheck, interval claim and
/// drain against [`flush`]. It is owned by a process ID rather than a parking
/// lot futex, so a child created by `fork` clears a lock inherited from a parent
/// thread that no longer exists instead of deadlocking on it.
/// [`record_drop`] remains an atomic increment and does not take that lock, and
/// a [`warn_if_due`] call whose interval is still open returns on a lock-free
/// read of the last-warning timestamp. Neither the clock nor the callbacks are
/// called while the lock is held: the clock is a caller-supplied seam, so a
/// clock that re-enters the warner must not find the claim's lock taken, and
/// callbacks are **not** serialized against one another and may re-enter the
/// warner. At most one [`warn_if_due`] caller claims an
/// interval: the claim uses compare-exchange, and a caller that loses it
/// returns without draining.
/// Concurrent callbacks therefore receive disjoint batches. A callback that
/// panics abandons the batch it was handed; the counter is not restored, which
/// keeps the "handed off once" guarantee rather than trading it for a
/// possibly-repeated callback.
///
/// The counter and timestamp use [`Ordering::Relaxed`], which is sound because
/// they publish only their own values and carry no data other than the count.
/// The lock uses acquire and release, so a forked child that reclaims it sees a
/// fully released critical section rather than a torn one.
///
/// [`record_drop`]: RateLimitedWarner::record_drop
/// [`warn_if_due`]: RateLimitedWarner::warn_if_due
/// [`flush`]: RateLimitedWarner::flush
pub struct RateLimitedWarner {
    /// Serializes due-window claims and counter drains against explicit flushes.
    drain_lock: ForkSafeDrainLock,
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
            drain_lock: ForkSafeDrainLock::new(),
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
    /// The counter is checked before sampling the clock and rechecked under the
    /// drain lock before the interval claim and drain. This serializes the due
    /// claim and drain against [`flush`], so a flush that drains the last
    /// pending batch cannot leave an empty interval claimed. At most one caller
    /// emits per interval; callers that lose the compare-exchange return without
    /// draining. The lock is released before the callback runs, so a slow or
    /// re-entrant callback does not hold up another caller or a flush. The clock
    /// is sampled before the lock is taken, so a re-entrant clock cannot wait
    /// for the claim it is timing.
    ///
    /// A call whose interval is still open returns before taking the lock. The
    /// refusal paths call this method for every dropped record, so under
    /// sustained overload the suppressed calls are the common case and must not
    /// contend for the shared lock. The pre-check reads `last_warn` without the
    /// lock, which is sound because the timestamp only moves forward: a stale
    /// read can only *over*-report that a warning is due, and the locked recheck
    /// corrects that; it can never hide a due warning.
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
        let count = self.claim_due_batch();
        if count > 0 {
            warn(count);
        }
    }

    /// Report whether a warning could be due, without taking the drain lock.
    ///
    /// Avoids clock reads for the common empty case, then compares the stale
    /// `last_warn` read against the interval. Suppressed calls are the common
    /// case on the refusal paths, so they must return before touching the
    /// shared lock. The read is sound without the lock because the timestamp
    /// only moves forward: a stale read can only claim a warning is due when it
    /// is not, and [`Self::claim_due_batch`] decides that under the lock.
    fn is_warning_due(&self) -> bool {
        if self.dropped.load(Ordering::Relaxed) == 0 {
            return false;
        }
        let now = self.clock.now_millis();
        let observed = self.last_warn.load(Ordering::Relaxed);
        observed == u64::MAX || now.saturating_sub(observed) >= self.interval_ms
    }

    /// Claim this interval and take the batch it covers, or return zero.
    ///
    /// The lock covers the recheck, the claim and the swap, so a concurrent
    /// [`Self::flush`] can neither empty a due batch after its interval has
    /// been claimed nor take the batch this call observes as pending. The
    /// caller re-reads nothing after this returns, and the callback runs with
    /// the lock released.
    ///
    /// The clock is sampled *before* the lock is taken. [`Clock`] is a seam the
    /// caller supplies, so a clock that re-enters the warner — a test clock
    /// that flushes, say — would otherwise wait forever for the lock this very
    /// call already holds. Sampling first is sound because the timestamp is
    /// only compared and then published: a caller that loses the interval to a
    /// concurrent claim observes a `last_warn` at or after its own sample and
    /// is refused by the locked recheck, and a caller whose sample is slightly
    /// stale claims at most a moment early, which the next call corrects.
    fn claim_due_batch(&self) -> u64 {
        let now = self.clock.now_millis();
        let _drain_guard = self.drain_lock.lock();
        // A concurrent flush may have taken the observed batch while the clock
        // was sampled, so do not claim an interval with nothing to report.
        if self.dropped.load(Ordering::Relaxed) == 0 {
            return 0;
        }
        let prev = self.last_warn.load(Ordering::Relaxed);
        if prev != u64::MAX && now.saturating_sub(prev) < self.interval_ms {
            return 0;
        }
        // `compare_exchange` ensures callers using the same observed timestamp
        // cannot both claim this warning interval.
        if !try_claim_interval(&self.last_warn, prev, now) {
            return 0;
        }
        self.dropped.swap(0, Ordering::Relaxed)
    }

    /// Immediately warn about any dropped records.
    ///
    /// This is not rate limited: it reports whatever is pending, even inside a
    /// suppression window, and does not consume or restart the interval. Use it
    /// on flush and shutdown paths, where the caller needs the count now rather
    /// than at the next due moment.
    ///
    /// The counter drain is serialized against the due check and interval
    /// claim. The lock is released before the callback runs.
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
#[path = "rate_limited_warner_concurrency_tests.rs"]
mod concurrency_tests;
