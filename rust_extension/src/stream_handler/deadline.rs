//! One absolute deadline shared by the two waits inside `flush`.
//!
//! `flush` first queues its command, which can block on a full channel, then
//! waits for the worker's acknowledgement. Giving each wait its own timeout
//! would let one call take nearly twice the configured limit, so both waits
//! read the time left before the same deadline.
//!
//! The deadline is measured on the handler's shared clock
//! ([`crate::rate_limited_warner::Clock`]) rather than with `Instant::now`,
//! so a test can advance it without sleeping and still exercise the real
//! channel underneath.

use std::time::Duration;

/// Converts a configured timeout to the millisecond resolution of the clock.
fn timeout_millis(timeout: Duration) -> u64 {
    u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX)
}

/// Absolute end of one `flush` call, measured in clock milliseconds.
pub(super) struct FlushDeadline {
    /// Clock reading, in milliseconds, by which both waits must have finished.
    expires_at_ms: u64,
}

impl FlushDeadline {
    /// Start a deadline one `timeout` after `now_ms`.
    pub(super) fn after(now_ms: u64, timeout: Duration) -> Self {
        Self {
            expires_at_ms: now_ms.saturating_add(timeout_millis(timeout)),
        }
    }

    /// Return the time left before the deadline; zero once it has passed.
    pub(super) fn remaining(&self, now_ms: u64) -> Duration {
        Duration::from_millis(self.expires_at_ms.saturating_sub(now_ms))
    }
}
