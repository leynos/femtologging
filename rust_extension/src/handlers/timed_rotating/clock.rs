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
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use chrono::{DateTime, TimeZone, Utc};
    use once_cell::sync::Lazy;

    static INJECTED_TIMES: Lazy<Mutex<VecDeque<i64>>> = Lazy::new(|| Mutex::new(VecDeque::new()));

    #[cfg(feature = "test-util")]
    pub(super) fn set(epoch_millis: Vec<i64>) {
        *INJECTED_TIMES
            .lock()
            .expect("timed rotation injected-time mutex poisoned") =
            epoch_millis.into_iter().collect();
    }

    #[cfg(feature = "test-util")]
    pub(super) fn clear() {
        INJECTED_TIMES
            .lock()
            .expect("timed rotation injected-time mutex poisoned")
            .clear();
    }

    pub(super) fn take() -> Option<DateTime<Utc>> {
        let mut guard = INJECTED_TIMES
            .lock()
            .expect("timed rotation injected-time mutex poisoned");
        guard
            .pop_front()
            .and_then(|epoch_millis| Utc.timestamp_millis_opt(epoch_millis).single())
    }
}

#[cfg(not(feature = "python"))]
mod injected {
    use chrono::{DateTime, Utc};

    pub(super) fn take() -> Option<DateTime<Utc>> {
        None
    }
}

fn take_injected_time() -> Option<DateTime<Utc>> {
    injected::take()
}

#[cfg(all(feature = "python", feature = "test-util"))]
pub(crate) fn set_injected_times_for_test(epoch_millis: Vec<i64>) {
    injected::set(epoch_millis);
}

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
