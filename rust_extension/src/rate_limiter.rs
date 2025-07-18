//! Rate-limited warning mechanism for dropped records.
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use log::warn;

pub type TimeProvider = Box<dyn Fn() -> u64 + Send + Sync>;

/// Issues rate-limited warnings about dropped records.
pub struct RateLimiter {
    last_warn: AtomicU64,
    dropped_records: AtomicU64,
    handler_name: String,
    warn_interval: u64,
    time_provider: TimeProvider,
}

impl RateLimiter {
    /// Create a new `RateLimiter` for the specified handler.
    pub fn new(handler_name: &str, warn_interval: u64, time_provider: TimeProvider) -> Self {
        Self {
            last_warn: AtomicU64::new(time_provider().saturating_sub(warn_interval)),
            dropped_records: AtomicU64::new(0),
            handler_name: handler_name.to_string(),
            warn_interval,
            time_provider,
        }
    }

    /// Increment the dropped record count and issue a warning if the rate
    /// limit has expired.
    pub fn record_dropped(&self) {
        self.dropped_records.fetch_add(1, Ordering::Relaxed);

        let now = (self.time_provider)();
        let prev = self.last_warn.load(Ordering::Relaxed);
        if now.saturating_sub(prev) >= self.warn_interval {
            self.report_dropped_records();
            self.last_warn.store(now, Ordering::Relaxed);
        }
    }

    /// Report the number of dropped records since the last interval.
    pub fn report_dropped_records(&self) {
        let dropped = self.dropped_records.swap(0, Ordering::Relaxed);
        if dropped > 0 {
            warn!(
                "{}: {} log records dropped in the last interval",
                self.handler_name, dropped
            );
        }
    }
}

/// Returns the current time in seconds since the UNIX epoch.
///
/// Returns 0 if the system clock is before the UNIX epoch.
pub fn system_time_provider() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
