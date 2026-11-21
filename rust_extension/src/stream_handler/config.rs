//! Configuration consumed by the stream handler lifecycle.
//!
//! `StreamHandlerBuilder` constructs these values before passing them to
//! [`FemtoStreamHandler`](super::FemtoStreamHandler) for runtime use.

use std::time::Duration;

use crate::monotonic_clock::{SharedClock, system_clock};
use crate::rate_limited_warner::{DEFAULT_WARN_INTERVAL, RateLimitedWarner};

/// Default queue capacity used when a stream builder does not provide one.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 1024;

/// Configuration for constructing a [`FemtoStreamHandler`](super::FemtoStreamHandler).
pub struct HandlerConfig {
    /// Bounded queue size, in commands, between callers and the worker. Records arriving
    /// while the queue is full are dropped.
    pub capacity: usize,
    /// Deadline shared by `flush`'s enqueue and acknowledgement waits, and the bound
    /// `close` waits for the worker's completion signal.
    pub flush_timeout: Duration,
    /// Rate limiter that throttles warnings about dropped records.
    pub warner: RateLimitedWarner,
    /// Clock the flush deadline is measured against, injected separately from
    /// the warner so a test can script the deadline without also changing when
    /// drop warnings are due.
    pub clock: SharedClock,
}

impl Default for HandlerConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CHANNEL_CAPACITY,
            flush_timeout: Duration::from_secs(1),
            warner: RateLimitedWarner::new(DEFAULT_WARN_INTERVAL),
            clock: system_clock(),
        }
    }
}

impl HandlerConfig {
    /// Set the worker queue capacity, in commands.
    pub fn with_capacity(mut self, capacity: usize) -> Self {
        self.capacity = capacity;
        self
    }

    /// Set the deadline shared by `flush`'s enqueue and acknowledgement waits.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.flush_timeout = timeout;
        self
    }

    /// Replace the clock the flush deadline is measured against.
    ///
    /// The injected clock is the same seam [`FlushContext::with_clock`] takes,
    /// so a test can script a handler's deadline and the barrier's context from
    /// one source.
    ///
    /// [`FlushContext::with_clock`]: crate::handler::FlushContext::with_clock
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let config = HandlerConfig::default().with_clock(scripted_clock());
    /// ```
    pub fn with_clock(mut self, clock: SharedClock) -> Self {
        self.clock = clock;
        self
    }

    /// Replace the rate limiter used for drop warnings. Available with the `test-util` feature.
    #[cfg(feature = "test-util")]
    pub fn with_warner(mut self, warner: RateLimitedWarner) -> Self {
        self.warner = warner;
        self
    }
}
