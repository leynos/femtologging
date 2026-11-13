//! Fixed-cardinality counters, the in-flight gauge, and their snapshot.
//!
//! Every counter is a relaxed atomic: the handler path must not pay for
//! cross-thread ordering, and a slightly stale snapshot is acceptable for
//! periodic reporting. The snapshot is a point-in-time copy, so a reader sees
//! one consistent key set even while writers keep recording.

use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use super::outcome::{OUTCOME_COUNT, WaitOutcome};
use super::timer::elapsed_micros;

/// Fixed-cardinality counters and duration totals for handler waits.
#[derive(Debug)]
pub(crate) struct WaitDiagnostics {
    /// One counter per [`WaitOutcome`].
    counts: [AtomicU64; OUTCOME_COUNT],
    /// Sum of every recorded wait, in microseconds.
    total_wait_micros: AtomicU64,
    /// Longest recorded wait, in microseconds.
    max_wait_micros: AtomicU64,
    /// Waits that have started and not yet recorded their outcome.
    in_flight: AtomicU64,
}

impl Default for WaitDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl WaitDiagnostics {
    /// Create a diagnostics set with every counter at zero.
    pub(crate) fn new() -> Self {
        Self {
            counts: std::array::from_fn(|_| AtomicU64::new(0)),
            total_wait_micros: AtomicU64::new(0),
            max_wait_micros: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
        }
    }

    /// Mark one wait as started and return a guard that marks it finished.
    ///
    /// The guard is held across the detached section, so a snapshot taken
    /// while the caller is blocked reports a non-zero in-flight count. Dropping
    /// it on every return path keeps the gauge correct when the wait unwinds,
    /// and `Relaxed` ordering is enough because the gauge is sampled, not used
    /// to synchronise anything.
    pub(crate) fn begin_wait(&self) -> WaitGuard<'_> {
        self.in_flight.fetch_add(1, Ordering::Relaxed);
        WaitGuard { diagnostics: self }
    }

    /// Record one wait and its outcome.
    pub(super) fn record(&self, outcome: WaitOutcome, elapsed: Duration) {
        // Relaxed ordering: these are independent counters read for periodic
        // reporting, so a slightly stale snapshot is acceptable and the
        // handler path must not pay for cross-thread ordering.
        let micros = elapsed_micros(elapsed);
        self.counts[outcome.index()].fetch_add(1, Ordering::Relaxed);
        self.total_wait_micros.fetch_add(micros, Ordering::Relaxed);
        self.max_wait_micros.fetch_max(micros, Ordering::Relaxed);
    }

    /// Take a consistent-enough snapshot of every counter.
    pub(crate) fn snapshot(&self) -> WaitSnapshot {
        let counts = std::array::from_fn(|index| self.counts[index].load(Ordering::Relaxed));
        WaitSnapshot {
            counts,
            total_wait_micros: self.total_wait_micros.load(Ordering::Relaxed),
            max_wait_micros: self.max_wait_micros.load(Ordering::Relaxed),
            in_flight: self.in_flight.load(Ordering::Relaxed),
        }
    }
}

/// Keeps the in-flight gauge correct for the lifetime of one wait.
///
/// An RAII guard rather than a paired increment and decrement so an early
/// return or a panic inside the detached section cannot leave the gauge stuck
/// above zero, which would make a healthy handler look permanently busy.
pub(crate) struct WaitGuard<'a> {
    /// Counters whose in-flight gauge this guard owns.
    diagnostics: &'a WaitDiagnostics,
}

impl Drop for WaitGuard<'_> {
    fn drop(&mut self) {
        self.diagnostics.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Point-in-time copy of a [`WaitDiagnostics`] set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WaitSnapshot {
    /// One count per [`WaitOutcome`], in the same index order.
    counts: [u64; OUTCOME_COUNT],
    /// Sum of every recorded wait, in microseconds.
    total_wait_micros: u64,
    /// Longest recorded wait, in microseconds.
    max_wait_micros: u64,
    /// Waits that had started and not yet recorded their outcome.
    in_flight: u64,
}

impl WaitSnapshot {
    /// Render the snapshot as the fixed key set exposed to Python.
    pub(crate) fn to_map(self) -> BTreeMap<&'static str, u64> {
        let mut map = BTreeMap::new();
        for outcome in WaitOutcome::ALL {
            map.insert(outcome.key(), self.count(outcome));
        }
        map.insert("total_wait_micros", self.total_wait_micros());
        map.insert("max_wait_micros", self.max_wait_micros());
        map.insert("waits_in_flight", self.in_flight());
        map
    }

    /// Return the count recorded for one outcome.
    pub(crate) fn count(self, outcome: WaitOutcome) -> u64 {
        self.counts[outcome.index()]
    }

    /// Return the summed wait time.
    pub(crate) fn total_wait_micros(self) -> u64 {
        self.total_wait_micros
    }

    /// Return the longest wait.
    pub(crate) fn max_wait_micros(self) -> u64 {
        self.max_wait_micros
    }

    /// Return the number of waits that had not yet finished.
    pub(crate) fn in_flight(self) -> u64 {
        self.in_flight
    }
}
