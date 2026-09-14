//! Clock abstractions for timed rotation.
//!
//! Production code reads the wall clock, while tests can inject deterministic
//! timestamps without sleeping.

use chrono::{DateTime, Utc};

/// Source of time for timed rotation decisions.
pub(crate) trait RotationClock: Send {
    /// Return the current instant in UTC.
    fn now(&mut self) -> DateTime<Utc>;
}

/// Wall-clock-backed time source used in production.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SystemClock;

impl RotationClock for SystemClock {
    fn now(&mut self) -> DateTime<Utc> {
        take_injected_time().unwrap_or_else(Utc::now)
    }
}

#[cfg(feature = "python")]
mod injected {
    //! Injected-time queue used by tests to make rotation deterministic.

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use chrono::{DateTime, TimeZone, Utc};
    use once_cell::sync::Lazy;

    /// Queues test-supplied epoch milliseconds behind a mutex so concurrent
    /// workers consume deterministic clock values safely.
    static INJECTED_TIMES: Lazy<Mutex<VecDeque<i64>>> = Lazy::new(|| Mutex::new(VecDeque::new()));

    /// Replaces the queued test times; available only to deterministic test
    /// builds.
    #[cfg(feature = "test-util")]
    pub(super) fn set(epoch_millis: Vec<i64>) {
        *INJECTED_TIMES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = epoch_millis.into_iter().collect();
    }

    /// Discards all queued test times before the next deterministic scenario.
    #[cfg(feature = "test-util")]
    pub(super) fn clear() {
        INJECTED_TIMES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    /// Pops one queued timestamp and converts it to UTC, recovering a poisoned
    /// mutex rather than losing the test clock.
    pub(super) fn take() -> Option<DateTime<Utc>> {
        let mut guard = INJECTED_TIMES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard
            .pop_front()
            .and_then(|epoch_millis| Utc.timestamp_millis_opt(epoch_millis).single())
    }
}

/// Provides the production fallback for builds without the Python test clock.
#[cfg(not(feature = "python"))]
mod injected {
    use chrono::{DateTime, Utc};

    /// Reports no injected time when test support is unavailable.
    pub(super) fn take() -> Option<DateTime<Utc>> {
        None
    }
}

/// Reads the next deterministic timestamp, or none when the queue is empty.
fn take_injected_time() -> Option<DateTime<Utc>> {
    injected::take()
}

/// Installs epoch-millisecond values for Python timed-rotation tests.
#[cfg(all(feature = "python", feature = "test-util"))]
pub(crate) fn set_injected_times_for_test(epoch_millis: Vec<i64>) {
    injected::set(epoch_millis);
}

/// Clears the Python test clock after a deterministic rotation scenario.
#[cfg(all(feature = "python", feature = "test-util"))]
pub(crate) fn clear_injected_times_for_test() {
    injected::clear();
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct SequenceClock {
    remaining: std::collections::VecDeque<DateTime<Utc>>,
    fallback: DateTime<Utc>,
}

#[cfg(test)]
impl SequenceClock {
    pub(crate) fn new(times: impl IntoIterator<Item = DateTime<Utc>>) -> Self {
        let remaining: std::collections::VecDeque<_> = times.into_iter().collect();
        let fallback = remaining
            .back()
            .cloned()
            .unwrap_or_else(|| Utc::now() + chrono::Duration::hours(1));
        Self {
            remaining,
            fallback,
        }
    }
}

#[cfg(test)]
impl RotationClock for SequenceClock {
    fn now(&mut self) -> DateTime<Utc> {
        self.remaining.pop_front().unwrap_or(self.fallback)
    }
}
