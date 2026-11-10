//! Scripted clocks for handler flush-deadline tests.
//!
//! Every built-in handler shares one budget between queue admission and the
//! worker's acknowledgement. Two properties follow from that, and neither is
//! observable against the wall clock: the supplied deadline — not the
//! handler's own timeout — must bound the wait, and a budget already spent on
//! admission must leave nothing for the acknowledgement. A scripted clock
//! makes both deterministic by returning a fixed sequence of samples.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::rate_limited_warner::Clock;

/// Deadline used by the tests, chosen so only the scripted offsets expire it.
///
/// Longer than any handler's own timeout, so a handler that ignored the
/// supplied deadline and used its configured budget instead would still wait
/// the whole time and be caught by the elapsed-time assertions.
const TEST_DEADLINE: Duration = Duration::from_secs(30);

/// A monotonic clock that returns one scripted offset per sample.
pub(crate) struct ScriptedClock {
    /// Time every sample is measured from.
    start: Instant,
    /// Offsets returned in order; the final offset repeats.
    offsets: Vec<Duration>,
    /// Number of samples requested so far.
    samples: usize,
}

impl ScriptedClock {
    /// Build a clock from millisecond offsets, one per sample.
    ///
    /// The last offset repeats, so a test that takes more samples than it
    /// scripted keeps seeing the final state rather than panicking.
    pub(crate) fn new(offsets_ms: &[u64]) -> Self {
        Self {
            start: Instant::now(),
            offsets: offsets_ms
                .iter()
                .copied()
                .map(Duration::from_millis)
                .collect(),
            samples: 0,
        }
    }

    /// The deadline every test passes to the handler.
    pub(crate) fn deadline(&self) -> Instant {
        self.start + TEST_DEADLINE
    }

    /// Return the next scripted sample.
    pub(crate) fn now(&mut self) -> Instant {
        let index = self.samples.min(self.offsets.len().saturating_sub(1));
        self.samples += 1;
        self.start + self.offsets[index]
    }

    /// Number of samples taken so far.
    ///
    /// A handler that consults the clock a different number of times is
    /// consuming a different budget, so the count is part of the assertion.
    pub(crate) fn samples(&self) -> usize {
        self.samples
    }
}

/// Warning clock the controlling test advances by hand.
///
/// A handler's warner rate-limits its drop warnings against this clock, so a
/// test can place a flush after the interval without sleeping.
#[derive(Default)]
pub(crate) struct WarningClock {
    /// Milliseconds elapsed since the clock was created.
    millis: AtomicU64,
}

impl WarningClock {
    /// Advance the clock by `by`.
    pub(crate) fn advance(&self, by: Duration) {
        self.millis
            .fetch_add(by.as_millis() as u64, Ordering::Relaxed);
    }
}

impl Clock for WarningClock {
    fn now_millis(&self) -> u64 {
        self.millis.load(Ordering::Relaxed)
    }
}
