use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// How often to emit warnings about dropped log records.
pub const WARN_RATE_LIMIT_SECS: u64 = 5;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Helper that rate limits dropped-record warnings.
///
/// The caller increments the drop counter via [`record_drop`]. The next call to
/// [`warn_if_due`] emits a warning using the provided callback if the configured
/// interval has elapsed. [`flush`] emits a warning immediately if any records
/// have been dropped since the last emission.
#[derive(Default)]
pub struct RateLimitedWarner {
    last_warn: AtomicU64,
    dropped: AtomicU64,
}

impl RateLimitedWarner {
    /// Create a new [`RateLimitedWarner`]. The first warning can be emitted
    /// immediately.
    pub fn new() -> Self {
        Self {
            last_warn: AtomicU64::new(now_secs().saturating_sub(WARN_RATE_LIMIT_SECS)),
            dropped: AtomicU64::new(0),
        }
    }

    /// Increment the dropped-record counter.
    pub fn record_drop(&self) {
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// Emit a warning if the rate limit interval has elapsed.
    pub fn warn_if_due(&self, mut warn: impl FnMut(u64)) {
        let now = now_secs();
        let prev = self.last_warn.load(Ordering::Relaxed);
        if now.saturating_sub(prev) >= WARN_RATE_LIMIT_SECS {
            let count = self.dropped.swap(0, Ordering::Relaxed);
            if count > 0 {
                warn(count);
            }
            self.last_warn.store(now, Ordering::Relaxed);
        }
    }

    /// Immediately warn about any dropped records.
    pub fn flush(&self, mut warn: impl FnMut(u64)) {
        let count = self.dropped.swap(0, Ordering::Relaxed);
        if count > 0 {
            warn(count);
            self.last_warn.store(now_secs(), Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn emits_first_warning_immediately() {
        let warner = RateLimitedWarner::new();
        let mut warnings = Vec::new();
        warner.record_drop();
        warner.warn_if_due(|c| warnings.push(c));
        assert_eq!(warnings, vec![1]);
    }

    #[test]
    fn rate_limits_subsequent_warnings() {
        let warner = RateLimitedWarner::new();
        let mut warnings = Vec::new();
        warner.record_drop();
        warner.warn_if_due(|c| warnings.push(c));
        warner.record_drop();
        warner.warn_if_due(|c| warnings.push(c));
        assert_eq!(warnings, vec![1]);
    }

    #[test]
    fn flush_emits_pending_warning() {
        let warner = RateLimitedWarner::new();
        let mut warnings = Vec::new();
        warner.record_drop();
        warner.flush(|c| warnings.push(c));
        assert_eq!(warnings, vec![1]);
    }
}
