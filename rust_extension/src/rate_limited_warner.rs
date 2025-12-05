//! Rate-limited warning system for dropped log records.
//!
//! This module provides [`RateLimitedWarner`], which tracks dropped log records
//! and emits warnings at configurable intervals to avoid spamming logs while
//! still alerting users to potential issues.

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
    last_warn: AtomicU64,
    dropped: AtomicU64,
    interval_ms: u64,
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
    pub fn warn_if_due(&self, mut warn: impl FnMut(u64)) {
        let now = self.clock.now_millis();
        let prev = self.last_warn.load(Ordering::Relaxed);
        if prev == u64::MAX || now.saturating_sub(prev) >= self.interval_ms {
            let count = self.dropped.swap(0, Ordering::Relaxed);
            if count > 0 {
                warn(count);
            }
            self.last_warn.store(now, Ordering::Relaxed);
        } else {
            self.dropped.store(0, Ordering::Relaxed);
        }
    }

    /// Immediately warn about any dropped records.
    pub fn flush(&self, warn: impl FnMut(u64)) {
        self.warn_if_due(warn);
    }
}

impl Default for RateLimitedWarner {
    fn default() -> Self {
        Self::new(DEFAULT_WARN_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::*;
    use std::sync::atomic::AtomicU64;

    struct FakeClock {
        now: AtomicU64,
    }

    impl Default for FakeClock {
        fn default() -> Self {
            Self {
                now: AtomicU64::new(0),
            }
        }
    }

    impl Clock for FakeClock {
        fn now_millis(&self) -> u64 {
            self.now.load(Ordering::Relaxed)
        }
    }

    impl FakeClock {
        fn advance(&self, ms: u64) {
            self.now.fetch_add(ms, Ordering::Relaxed);
        }
    }

    #[fixture]
    fn clock() -> Arc<FakeClock> {
        Arc::new(FakeClock::default())
    }

    #[fixture]
    fn warner() -> (RateLimitedWarner, Arc<FakeClock>) {
        let clock = Arc::new(FakeClock::default());
        (
            RateLimitedWarner::with_clock(Duration::from_secs(1), clock.clone() as Arc<dyn Clock>),
            clock,
        )
    }

    #[fixture]
    fn warnings() -> Vec<u64> {
        Vec::new()
    }

    #[rstest]
    #[case(1)]
    #[case(3)]
    fn accumulates_drops_and_emits_warning(
        warner: (RateLimitedWarner, Arc<FakeClock>),
        mut warnings: Vec<u64>,
        #[case] drop_count: u64,
    ) {
        let (warner, _clock) = warner;
        for _ in 0..drop_count {
            warner.record_drop();
        }
        warner.warn_if_due(|c| warnings.push(c));
        assert_eq!(warnings, vec![drop_count]);
    }

    #[rstest]
    fn rate_limits_subsequent_warnings(
        mut warnings: Vec<u64>,
        warner: (RateLimitedWarner, Arc<FakeClock>),
    ) {
        let (warner, clock) = warner;
        warner.record_drop();
        warner.warn_if_due(|c| warnings.push(c));
        warner.record_drop();
        warner.warn_if_due(|c| warnings.push(c));
        assert_eq!(warnings, vec![1]);
        clock.advance(1000);
        warner.record_drop();
        warner.warn_if_due(|c| warnings.push(c));
        assert_eq!(warnings, vec![1, 1]);
    }

    #[rstest]
    fn flush_emits_pending_warning(
        warner: (RateLimitedWarner, Arc<FakeClock>),
        mut warnings: Vec<u64>,
    ) {
        let (warner, _) = warner;
        warner.record_drop();
        warner.flush(|c| warnings.push(c));
        assert_eq!(warnings, vec![1]);
    }

    #[rstest]
    fn no_warning_when_no_drops(
        warner: (RateLimitedWarner, Arc<FakeClock>),
        mut warnings: Vec<u64>,
    ) {
        let (warner, _) = warner;
        warner.warn_if_due(|c| warnings.push(c));
        assert!(warnings.is_empty());
    }

    #[rstest]
    fn flush_with_no_drops_does_nothing(
        warner: (RateLimitedWarner, Arc<FakeClock>),
        mut warnings: Vec<u64>,
    ) {
        let (warner, _) = warner;
        warner.flush(|c| warnings.push(c));
        assert!(warnings.is_empty());
    }
}
