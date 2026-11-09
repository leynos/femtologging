//! Exponential backoff state machine used by the socket worker.

use std::time::{Duration, Instant};

#[cfg(test)]
use rand::SeedableRng;
use rand::{RngExt, rngs::StdRng};

use super::config::BackoffPolicy;

/// Tracks reconnection attempts and produces jittered delays.
pub struct BackoffState {
    /// Retry policy supplying base, cap, reset, and deadline limits.
    policy: BackoffPolicy,
    /// Current exponential delay, reset after a successful or idle connection.
    current: Duration,
    /// Start of the current failure window, used to enforce the retry deadline.
    failure_since: Option<Instant>,
    /// Worker-owned random source used to jitter each retry delay.
    rng: StdRng,
    /// Most recent successful write; sustained idleness restores initial state.
    last_success: Option<Instant>,
}

impl BackoffState {
    /// Create a new state machine from the supplied policy.
    ///
    /// The random source is seeded from the operating system through
    /// [`rand::make_rng`]. That function panics if the OS entropy source is
    /// unavailable, which preserves the previous `StdRng::from_entropy`
    /// behaviour and is deliberate: the worker cannot produce unpredictable
    /// jitter without entropy, and a fixed-seed fallback would make every
    /// process retry in lockstep.
    pub fn new(policy: BackoffPolicy) -> Self {
        Self::with_rng(policy, rand::make_rng())
    }

    /// Build state around an already-created random source.
    fn with_rng(policy: BackoffPolicy, rng: StdRng) -> Self {
        Self {
            current: policy.base,
            failure_since: None,
            rng,
            last_success: None,
            policy,
        }
    }

    /// Build state with a deterministic seed for unit tests.
    ///
    /// `seed_from_u64` is not suitable for cryptographic use, but jitter only
    /// needs to spread retries. Tests use it so the same seed and operations
    /// produce the same delays within one build without asserting a particular
    /// `StdRng` stream, which is not stable across rand versions.
    #[cfg(test)]
    pub(crate) fn with_seed(policy: BackoffPolicy, seed: u64) -> Self {
        Self::with_rng(policy, StdRng::seed_from_u64(seed))
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
            _ => self.rng.random_range(MIN_SLEEP_MS..=max_ms),
        };
        Some(Duration::from_millis(sleep_ms))
    }
}
