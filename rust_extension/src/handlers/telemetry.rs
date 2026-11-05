//! Bounded diagnostics for the Python-facing file-handler waits.
//!
//! The three Python entry points that may block — `handle()`, `flush()`, and
//! `close()` — record one counter per outcome and accumulate their wait
//! duration here. The counter set is closed: it never carries logger names,
//! paths, messages, or raw timeout values, so an operator can scrape it without
//! unbounded cardinality. `FemtoFileHandler.wait_diagnostics()` exposes a
//! snapshot as a Python dictionary, and each record also emits an internal
//! `tracing` event when the `tracing-compat` feature is enabled.
//!
//! Events target `femtologging::internal`, which `FemtoTracingLayer` ignores so
//! that instrumentation cannot recursively log about itself. Every field is
//! bounded: operation, handler kind, policy bucket, outcome, and elapsed
//! microseconds.

use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

/// Number of outcomes a handler wait can report.
pub(crate) const OUTCOME_COUNT: usize = 12;

/// Outcome of one Python-facing handler wait.
///
/// The variants are per operation so the counter key can be derived without
/// consulting the operation name; the closed set is what keeps the exposed
/// dictionary fixed-cardinality.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WaitOutcome {
    /// `handle()` enqueued the record.
    HandleAccepted,
    /// `handle()` found the queue full under the drop policy.
    HandleQueueFull,
    /// `handle()` reached its timeout without enqueueing.
    HandleTimedOut,
    /// `handle()` found the handler already closed or its channel disconnected.
    HandleClosed,
    /// `handle()` failed for a reason the handler cannot classify further.
    HandleFailed,
    /// `flush()` received the worker's acknowledgement.
    FlushAcknowledged,
    /// `flush()` timed out before the worker acknowledged.
    FlushTimedOut,
    /// `flush()` found the handler already closed or its channel disconnected.
    FlushClosed,
    /// The worker answered the flush with an I/O failure.
    FlushFailed,
    /// `close()` observed the worker stop.
    CloseCompleted,
    /// `close()` reached its one-second bound without the worker stopping.
    CloseTimedOut,
    /// `close()` joined a worker that had panicked.
    ClosePanicked,
}

impl WaitOutcome {
    /// Every outcome, in counter-index order.
    const ALL: [Self; OUTCOME_COUNT] = [
        Self::HandleAccepted,
        Self::HandleQueueFull,
        Self::HandleTimedOut,
        Self::HandleClosed,
        Self::HandleFailed,
        Self::FlushAcknowledged,
        Self::FlushTimedOut,
        Self::FlushClosed,
        Self::FlushFailed,
        Self::CloseCompleted,
        Self::CloseTimedOut,
        Self::ClosePanicked,
    ];

    /// Stable key used in the Python dictionary and the tracing event.
    ///
    /// The operation prefix is part of the key so a single flat dictionary
    /// cannot confuse a `handle()` outcome with a `flush()` outcome.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::HandleAccepted => "handle_accepted",
            Self::HandleQueueFull => "handle_queue_full",
            Self::HandleTimedOut => "handle_timed_out",
            Self::HandleClosed => "handle_closed",
            Self::HandleFailed => "handle_failed",
            Self::FlushAcknowledged => "flush_acknowledged",
            Self::FlushTimedOut => "flush_timed_out",
            Self::FlushClosed => "flush_closed",
            Self::FlushFailed => "flush_failed",
            Self::CloseCompleted => "close_completed",
            Self::CloseTimedOut => "close_timed_out",
            Self::ClosePanicked => "close_panicked",
        }
    }

    /// Position of the outcome in the counter array.
    fn index(self) -> usize {
        self as usize
    }
}

/// Bounded context attached to one recorded wait.
#[derive(Clone, Copy)]
pub(crate) struct WaitContext {
    /// Operation name such as `handler_flush`.
    pub(crate) operation: &'static str,
    /// Handler category such as `file` or `timed_rotating`.
    pub(crate) handler_kind: &'static str,
    /// Overflow policy bucket: `drop`, `block`, or `timeout`.
    pub(crate) policy_bucket: &'static str,
}

/// Fixed-cardinality counters and duration totals for handler waits.
#[derive(Debug)]
pub(crate) struct WaitDiagnostics {
    /// One counter per [`WaitOutcome`].
    counts: [AtomicU64; OUTCOME_COUNT],
    /// Sum of every recorded wait, in microseconds.
    total_wait_micros: AtomicU64,
    /// Longest recorded wait, in microseconds.
    max_wait_micros: AtomicU64,
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
        }
    }

    /// Record one wait and its outcome.
    fn record(&self, outcome: WaitOutcome, elapsed: Duration) {
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
        }
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
}

/// Times one Python-facing wait and records its outcome when it returns.
pub(crate) struct WaitTimer {
    /// Instant captured immediately before the blocking call.
    started: Instant,
}

impl WaitTimer {
    /// Start timing one wait.
    pub(crate) fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }

    /// Stop timing, record the outcome, and emit the internal event.
    ///
    /// Call this only after the detached section has returned, so the event is
    /// emitted on the interpreter-owning thread and never prompts the tracing
    /// bridge to attach to Python from a GIL-free thread.
    pub(crate) fn finish(
        self,
        diagnostics: &WaitDiagnostics,
        context: WaitContext,
        outcome: WaitOutcome,
    ) {
        let elapsed = self.started.elapsed();
        diagnostics.record(outcome, elapsed);
        emit_event(context, outcome, elapsed);
    }
}

/// Convert a wait duration to the microseconds the counters and events carry.
///
/// Saturating rather than wrapping: a wait longer than about 584 000 years is
/// not meaningful, and reporting `u64::MAX` is better than an apparent small
/// value.
fn elapsed_micros(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

/// Emit the bounded internal tracing event for one wait.
#[cfg(feature = "tracing-compat")]
fn emit_event(context: WaitContext, outcome: WaitOutcome, elapsed: Duration) {
    tracing::trace!(
        target: "femtologging::internal",
        operation = context.operation,
        handler_kind = context.handler_kind,
        policy_bucket = context.policy_bucket,
        wait_outcome = outcome.key(),
        elapsed_us = elapsed_micros(elapsed),
    );
}

/// Keep the bounded event fields live when the tracing feature is off.
///
/// The counters remain the only observable output in that build, but the
/// fields are still part of the event contract, so reference them instead of
/// letting dead-code analysis report the context as unused.
#[cfg(not(feature = "tracing-compat"))]
fn emit_event(context: WaitContext, outcome: WaitOutcome, elapsed: Duration) {
    let _ = (
        context.operation,
        context.handler_kind,
        context.policy_bucket,
        outcome,
        elapsed,
    );
}

#[cfg(test)]
mod tests {
    //! Unit tests for the counter table, snapshots, and the wait timer.

    use super::*;
    use std::collections::HashSet;

    /// Record one outcome with an explicit duration, bypassing the clock.
    fn record_with(diagnostics: &WaitDiagnostics, outcome: WaitOutcome, micros: u64) {
        diagnostics.record(outcome, Duration::from_micros(micros));
    }

    #[test]
    fn every_outcome_has_a_unique_key() {
        let keys: HashSet<&'static str> = WaitOutcome::ALL.iter().map(|o| o.key()).collect();
        assert_eq!(keys.len(), OUTCOME_COUNT, "outcome keys must be unique");
    }

    #[test]
    fn the_exposed_map_has_a_fixed_key_set() {
        let map = WaitDiagnostics::new().snapshot().to_map();
        assert_eq!(map.len(), OUTCOME_COUNT + 2, "counters plus two totals");
        for outcome in WaitOutcome::ALL {
            assert!(map.contains_key(outcome.key()), "missing {}", outcome.key());
        }
        assert!(map.contains_key("total_wait_micros"));
        assert!(map.contains_key("max_wait_micros"));
    }

    #[test]
    fn recording_increments_only_the_named_counter() {
        let diagnostics = WaitDiagnostics::new();
        record_with(&diagnostics, WaitOutcome::HandleAccepted, 10);
        record_with(&diagnostics, WaitOutcome::HandleAccepted, 10);
        record_with(&diagnostics, WaitOutcome::FlushTimedOut, 10);
        let snapshot = diagnostics.snapshot();

        assert_eq!(snapshot.count(WaitOutcome::HandleAccepted), 2);
        assert_eq!(snapshot.count(WaitOutcome::FlushTimedOut), 1);
        assert_eq!(snapshot.count(WaitOutcome::CloseCompleted), 0);
    }

    #[test]
    fn durations_accumulate_and_track_the_longest_wait() {
        let diagnostics = WaitDiagnostics::new();
        record_with(&diagnostics, WaitOutcome::HandleAccepted, 1_000);
        record_with(&diagnostics, WaitOutcome::CloseCompleted, 5_000);
        let snapshot = diagnostics.snapshot();

        assert_eq!(snapshot.total_wait_micros(), 6_000);
        assert_eq!(snapshot.max_wait_micros(), 5_000);
        assert!(snapshot.total_wait_micros() >= snapshot.max_wait_micros());
    }

    #[test]
    fn the_timer_records_exactly_one_wait() {
        let diagnostics = WaitDiagnostics::new();
        let timer = WaitTimer::start();
        timer.finish(
            &diagnostics,
            WaitContext {
                operation: "handler_flush",
                handler_kind: "file",
                policy_bucket: "drop",
            },
            WaitOutcome::FlushAcknowledged,
        );
        let snapshot = diagnostics.snapshot();
        assert_eq!(snapshot.count(WaitOutcome::FlushAcknowledged), 1);
        assert_eq!(snapshot.total_wait_micros(), snapshot.max_wait_micros());
    }

    #[test]
    fn outcomes_index_the_counter_array_without_gaps() {
        // `WaitOutcome` is cast to `usize`, so a gap would silently alias two
        // outcomes onto one counter.
        for (index, outcome) in WaitOutcome::ALL.iter().enumerate() {
            assert_eq!(
                outcome.index(),
                index,
                "{} must sit at {index}",
                outcome.key()
            );
        }
    }
}
