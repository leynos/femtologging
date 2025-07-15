//! Rate-limited warning mechanism for dropped records.
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use log::warn;

const WARN_RATE_LIMIT_SECS: u64 = 5;

/// Issues rate-limited warnings about dropped records.
pub struct RateLimiter {
    last_warn: AtomicU64,
    dropped_records: Arc<Mutex<u64>>,
    handler_name: String,
}

impl RateLimiter {
    /// Create a new `RateLimiter` for the specified handler.
    pub fn new(handler_name: &str) -> Self {
        Self {
            last_warn: AtomicU64::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .saturating_sub(WARN_RATE_LIMIT_SECS),
            ),
            dropped_records: Arc::new(Mutex::new(0)),
            handler_name: handler_name.to_string(),
        }
    }

    /// Increment the dropped record count and issue a warning if the rate
    /// limit has expired.
    pub fn record_dropped(&self) {
        {
            let mut dropped = self.dropped_records.lock().unwrap();
            *dropped += 1;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let prev = self.last_warn.load(Ordering::Relaxed);
        if now.saturating_sub(prev) >= WARN_RATE_LIMIT_SECS {
            self.report_dropped_records();
            self.last_warn.store(now, Ordering::Relaxed);
        }
    }

    /// Report the number of dropped records since the last interval.
    pub fn report_dropped_records(&self) {
        let mut dropped = self.dropped_records.lock().unwrap();
        if *dropped > 0 {
            warn!(
                "{}: {} log records dropped in the last interval",
                self.handler_name, *dropped
            );
            *dropped = 0;
        }
    }
}
