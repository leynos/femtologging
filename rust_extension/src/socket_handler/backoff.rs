//! Exponential backoff state machine used by the socket worker.

use std::time::{Duration, Instant};

use rand::{Rng, SeedableRng, rngs::StdRng};

use super::config::BackoffPolicy;

/// Tracks reconnection attempts and produces jittered delays.
pub struct BackoffState {
    policy: BackoffPolicy,
    current: Duration,
    failure_since: Option<Instant>,
    rng: StdRng,
    last_success: Option<Instant>,
}

impl BackoffState {
    /// Create a new state machine from the supplied policy.
    pub fn new(policy: BackoffPolicy) -> Self {
        Self {
            current: policy.base,
            failure_since: None,
            rng: StdRng::from_entropy(),
            last_success: None,
            policy,
        }
    }

    /// Record a successful write event.
    pub fn record_success(&mut self, now: Instant) {
        self.last_success = Some(now);
        if let Some(start) = self.failure_since
            && now.duration_since(start) >= self.policy.reset_after
        {
            self.current = self.policy.base;
            self.failure_since = None;
        }
    }

    /// Reset the backoff window after a sustained period without failures.
    pub fn reset_after_idle(&mut self, now: Instant) {
        if let Some(success) = self.last_success
            && now.duration_since(success) >= self.policy.reset_after
        {
            self.current = self.policy.base;
            self.failure_since = None;
            self.last_success = None;
        }
    }

    /// Calculate the next jittered sleep duration following a failure.
    pub fn next_sleep(&mut self, now: Instant) -> Option<Duration> {
        let start = self.failure_since.unwrap_or_else(|| {
            self.failure_since = Some(now);
            now
        });

        if now.duration_since(start) >= self.policy.deadline {
            return None;
        }

        if now != start {
            self.current = self.current.saturating_mul(2).min(self.policy.cap);
        }

        const MIN_SLEEP_MS: u64 = 10;

        let max_ms = self.current.as_millis().min(u128::from(u64::MAX)) as u64;
        let sleep_ms = match max_ms {
            0 => MIN_SLEEP_MS,
            1..=MIN_SLEEP_MS => max_ms,
            _ => self.rng.gen_range(MIN_SLEEP_MS..=max_ms),
        };
        Some(Duration::from_millis(sleep_ms))
    }
}
