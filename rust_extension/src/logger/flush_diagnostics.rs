//! Fixed-cardinality accounting for logger flush outcomes and durations.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Fixed set of terminal failure stages for the flush barrier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum FlushFailureStage {
    /// The configured logger timeout could not form a monotonic deadline.
    DeadlineUnrepresentable,
    /// Process-local dispatch state could not be resolved before the deadline.
    DispatchStateTimedOut,
    /// The logger worker sender was no longer available.
    WorkerUnavailable,
    /// The handler registry read lock was unavailable before the deadline.
    HandlerSnapshotTimedOut,
    /// The deadline expired before the marker could be enqueued.
    DeadlineExpiredBeforeMarker,
    /// The bounded logger queue remained full until the deadline.
    MarkerEnqueueTimedOut,
    /// The logger worker had disconnected before accepting the marker.
    MarkerEnqueueDisconnected,
    /// The accepted marker did not pass its start gate before the deadline.
    WorkerStartTimedOut,
    /// The worker dropped the accepted marker before starting its flush.
    WorkerStartDisconnected,
    /// A handler returned false or did not support deadline-aware flushing.
    HandlerFlushUnconfirmed,
    /// A handler callback returned after the shared deadline.
    HandlerFlushDeadlineExceeded,
    /// The worker did not acknowledge the marker before the deadline.
    AcknowledgementTimedOut,
    /// The worker exited before acknowledging the marker.
    WorkerExited,
    /// The barrier completed after its absolute deadline.
    BarrierDeadlineExceeded,
    /// Interpreter reattachment or the final native return missed the deadline.
    CallerReturnDeadlineExceeded,
}

impl FlushFailureStage {
    /// Number of fixed failure-stage counters in [`FlushDiagnostics`].
    const COUNT: usize = Self::CallerReturnDeadlineExceeded as usize + 1;
}

/// Per-logger, fixed-cardinality accounting for flush attempts.
///
/// Recording touches only relaxed atomics. Exporting these counters through a
/// supported metrics interface remains tracked by issue `#551`.
pub(super) struct FlushDiagnostics {
    /// Number of completed flush attempts.
    attempts: AtomicU64,
    /// Number of barriers confirmed by their caller deadline.
    confirmed: AtomicU64,
    /// Terminal failures indexed by [`FlushFailureStage`].
    failures: [AtomicU64; FlushFailureStage::COUNT],
    /// Elapsed-time samples in five fixed buckets: 10, 100, 500, and 2,000 ms,
    /// followed by values above 2,000 ms.
    elapsed_buckets: [AtomicU64; 5],
}

impl Default for FlushDiagnostics {
    fn default() -> Self {
        Self {
            attempts: AtomicU64::new(0),
            confirmed: AtomicU64::new(0),
            failures: std::array::from_fn(|_| AtomicU64::new(0)),
            elapsed_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl FlushDiagnostics {
    /// Account for one terminal outcome without calling a logging callback.
    pub(super) fn record(&self, outcome: Result<(), FlushFailureStage>, elapsed: Duration) {
        self.attempts.fetch_add(1, Ordering::Relaxed);
        match outcome {
            Ok(()) => {
                self.confirmed.fetch_add(1, Ordering::Relaxed);
            }
            Err(stage) => {
                self.failures[stage as usize].fetch_add(1, Ordering::Relaxed);
            }
        }
        self.elapsed_buckets[elapsed_bucket(elapsed)].fetch_add(1, Ordering::Relaxed);
    }

    /// Snapshot the fixed counters for focused unit tests.
    #[cfg(all(test, not(loom)))]
    pub(super) fn snapshot(&self) -> FlushDiagnosticsSnapshot {
        FlushDiagnosticsSnapshot {
            attempts: self.attempts.load(Ordering::Relaxed),
            confirmed: self.confirmed.load(Ordering::Relaxed),
            failures: self
                .failures
                .each_ref()
                .map(|count| count.load(Ordering::Relaxed)),
            elapsed_buckets: self
                .elapsed_buckets
                .each_ref()
                .map(|count| count.load(Ordering::Relaxed)),
        }
    }
}

/// Read-only diagnostic values returned to tests.
#[cfg(all(test, not(loom)))]
pub(super) struct FlushDiagnosticsSnapshot {
    /// Number of completed flush attempts.
    pub(super) attempts: u64,
    /// Number of barriers confirmed by their caller deadline.
    pub(super) confirmed: u64,
    /// Terminal failures indexed by [`FlushFailureStage`].
    pub(super) failures: [u64; FlushFailureStage::COUNT],
    /// Elapsed-time counts for the five fixed buckets.
    pub(super) elapsed_buckets: [u64; 5],
}

/// Return the fixed elapsed-time bucket used by flush diagnostics.
pub(super) fn elapsed_bucket(elapsed: Duration) -> usize {
    match elapsed.as_millis() {
        0..=10 => 0,
        11..=100 => 1,
        101..=500 => 2,
        501..=2_000 => 3,
        _ => 4,
    }
}
