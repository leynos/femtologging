//! Configuration consumed by the stream handler lifecycle.
//!
//! `StreamHandlerBuilder` constructs these values before passing them to
//! [`FemtoStreamHandler`](super::FemtoStreamHandler) for runtime use.

use std::time::Duration;

use crate::rate_limited_warner::{DEFAULT_WARN_INTERVAL, RateLimitedWarner};

/// Default queue capacity used when a stream builder does not provide one.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Configuration for constructing a [`FemtoStreamHandler`](super::FemtoStreamHandler).
pub struct HandlerConfig {
    pub capacity: usize,
    pub flush_timeout: Duration,
    pub warner: RateLimitedWarner,
}

impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_timeout: Duration::from_secs(1),
            warner: RateLimitedWarner::new(DEFAULT_WARN_INTERVAL),
        }
    }
}

impl HandlerConfig {
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.flush_timeout = timeout;
        self
    }

    #[cfg(feature = "test-util")]
    pub fn with_warner(mut self, warner: RateLimitedWarner) -> Self {
        self.warner = warner;
        self
    }
}
