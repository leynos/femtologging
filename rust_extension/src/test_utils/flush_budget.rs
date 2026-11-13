//! Scripted clocks for handler flush-deadline tests.
//!
//! Every built-in handler shares one budget between queue admission and the
//! worker's acknowledgement. Two properties follow from that, and neither is
//! observable against the wall clock: the supplied deadline — not the
//! handler's own timeout — must bound the wait, and a budget already spent on
//! admission must leave nothing for the acknowledgement. A scripted clock
//! makes both deterministic by returning a fixed sequence of samples.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::monotonic_clock::MonotonicClock;
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

    /// The instant every scripted offset is measured from.
    pub(crate) fn started_at(&self) -> Instant {
        self.start
    }

    /// The deadline a caller with `budget` forms at this clock's start.
    pub(crate) fn deadline_within(&self, budget: Duration) -> Instant {
        self.start + budget
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

/// A [`ScriptedClock`] that a [`FlushContext`] can hold.
///
/// `flush_until` receives its time source inside the context rather than as a
/// closure, so a test that exercises that entry point needs a clock the context
/// can sample. This wrapper keeps the scripted sequence behind a lock and
/// reports it through [`MonotonicClock`], which is the trait the context reads.
/// The samples are drawn in the same order as the closure form, so a test can
/// assert the same sample counts it would for `flush_by_at`.
///
/// [`FlushContext`]: crate::handler::FlushContext
pub(crate) struct SharedScriptedClock {
    /// The scripted samples, taken one per request.
    inner: Mutex<ScriptedClock>,
}

impl SharedScriptedClock {
    /// Build a shared clock from millisecond offsets, one per sample.
    pub(crate) fn new(offsets_ms: &[u64]) -> Self {
        Self {
            inner: Mutex::new(ScriptedClock::new(offsets_ms)),
        }
    }

    /// The instant every scripted offset is measured from.
    pub(crate) fn started_at(&self) -> Instant {
        self.lock().started_at()
    }

    /// The deadline a caller with `budget` forms at this clock's start.
    pub(crate) fn deadline_within(&self, budget: Duration) -> Instant {
        self.lock().deadline_within(budget)
    }

    /// Number of samples taken so far.
    pub(crate) fn samples(&self) -> usize {
        self.lock().samples()
    }

    /// Take the scripted clock's lock, recovering from a poisoned mutex.
    ///
    /// A panic in another test thread cannot leave the sample sequence invalid:
    /// every method either reads or appends one sample, so the recovery is
    /// sound and keeps a failure in one test from cascading into the next.
    fn lock(&self) -> std::sync::MutexGuard<'_, ScriptedClock> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl MonotonicClock for SharedScriptedClock {
    fn now(&self) -> Instant {
        self.lock().now()
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
