//! Fixed-cardinality accounting for logger flush outcomes and durations.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Fixed set of terminal failure stages for the flush barrier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum FlushFailureStage {
    /// The configured logger timeout could not form a monotonic deadline.
    DeadlineUnrepresentable,
    /// Process-local dispatch state could not be resolved before the deadline.
    DispatchStateTimedOut,
    /// The logger worker sender was no longer available.
    WorkerUnavailable,
    /// The handler registry lock and snapshot handoff missed the deadline.
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

    /// Every terminal failure in the stable diagnostic schema.
    const ALL: [Self; Self::COUNT] = [
        Self::DeadlineUnrepresentable,
        Self::DispatchStateTimedOut,
        Self::WorkerUnavailable,
        Self::HandlerSnapshotTimedOut,
        Self::DeadlineExpiredBeforeMarker,
        Self::MarkerEnqueueTimedOut,
        Self::MarkerEnqueueDisconnected,
        Self::WorkerStartTimedOut,
        Self::WorkerStartDisconnected,
        Self::HandlerFlushUnconfirmed,
        Self::HandlerFlushDeadlineExceeded,
        Self::AcknowledgementTimedOut,
        Self::WorkerExited,
        Self::BarrierDeadlineExceeded,
        Self::CallerReturnDeadlineExceeded,
    ];

    /// Stable low-cardinality suffix for the production diagnostic key.
    const fn metric_suffix(self) -> &'static str {
        match self {
            Self::DeadlineUnrepresentable => "deadline_unrepresentable",
            Self::DispatchStateTimedOut => "dispatch_state_timed_out",
            Self::WorkerUnavailable => "worker_unavailable",
            Self::HandlerSnapshotTimedOut => "handler_snapshot_timed_out",
            Self::DeadlineExpiredBeforeMarker => "deadline_expired_before_marker",
            Self::MarkerEnqueueTimedOut => "marker_enqueue_timed_out",
            Self::MarkerEnqueueDisconnected => "marker_enqueue_disconnected",
            Self::WorkerStartTimedOut => "worker_start_timed_out",
            Self::WorkerStartDisconnected => "worker_start_disconnected",
            Self::HandlerFlushUnconfirmed => "handler_flush_unconfirmed",
            Self::HandlerFlushDeadlineExceeded => "handler_flush_deadline_exceeded",
            Self::AcknowledgementTimedOut => "acknowledgement_timed_out",
            Self::WorkerExited => "worker_exited",
            Self::BarrierDeadlineExceeded => "barrier_deadline_exceeded",
            Self::CallerReturnDeadlineExceeded => "caller_return_deadline_exceeded",
        }
    }
}

/// Trace a worker-side flush result with bounded, non-sensitive fields.
#[cfg(feature = "tracing-compat")]
pub(super) fn trace_flush_result(
    event: &'static str,
    result: Result<(), FlushFailureStage>,
    elapsed: Duration,
) {
    let (outcome, failure_stage) = match result {
        Ok(()) => ("confirmed", "none"),
        Err(stage) => ("unconfirmed", stage.metric_suffix()),
    };
    tracing::trace!(
        target: "femtologging::internal",
        operation = "flush_handlers",
        event,
        outcome,
        failure_stage,
        elapsed_us = elapsed.as_micros(),
    );
}

/// Per-logger, fixed-cardinality accounting for flush attempts.
///
/// Recording touches only relaxed atomics. Exporting these counters through a
/// shared metrics backend remains tracked by issue `#551`.
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

    /// Read a best-effort snapshot of the fixed counters.
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

/// Read-only, fixed-cardinality diagnostic values for a logger.
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

impl FlushDiagnosticsSnapshot {
    /// Return stable metric names with no logger-specific dimensions.
    pub(super) fn metrics(&self) -> BTreeMap<String, u64> {
        let mut metrics = BTreeMap::new();
        metrics.insert("flush_handlers_attempts".to_owned(), self.attempts);
        metrics.insert("flush_handlers_confirmed".to_owned(), self.confirmed);
        for stage in FlushFailureStage::ALL {
            metrics.insert(
                format!("flush_handlers_failure_{}", stage.metric_suffix()),
                self.failures[stage as usize],
            );
        }
        for (bucket, suffix) in [
            "le_10_ms",
            "le_100_ms",
            "le_500_ms",
            "le_2000_ms",
            "gt_2000_ms",
        ]
        .into_iter()
        .enumerate()
        {
            metrics.insert(
                format!("flush_handlers_elapsed_{suffix}"),
                self.elapsed_buckets[bucket],
            );
        }
        metrics
    }
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
