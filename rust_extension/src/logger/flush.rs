//! Deadline-aware handler flushing for [`super::FemtoLogger`].
//!
//! A flush has two halves: the logger's own queued records must be handled
//! before the handlers are asked to flush, and the handlers must then be
//! flushed without holding any lock a handler could need. One absolute
//! deadline spans dispatch resolution, the shard's flush marker, the handler
//! snapshot, the worker's acknowledgement, and each handler's own flush, so a
//! caller is bounded once rather than once per stage. Built-in handlers keep
//! their shorter per-handler timeouts, and a caller that holds Python access
//! releases the interpreter for the wait.
//!
//! This module owns the barrier and its outcome. The measurement seam lives in
//! [`super::flush_measure`], and the handler-snapshot sequence in
//! [`super::flush_snapshot`]. The barrier keeps a richer outcome of its own —
//! the stage that failed and the deadline it was charged against — and is
//! collapsed to the four metric buckets only for the counters.

use std::sync::Arc;
use std::time::{Duration, Instant};

use log::warn;

use crate::handler::FemtoHandlerTrait;
#[cfg(all(test, not(loom)))]
use crate::monotonic_clock::MonotonicClock;
#[cfg(all(test, not(loom)))]
use crate::monotonic_clock::SharedClock;
use crate::sync::Sender;

use super::flush_clock::FlushDeadline;
use super::flush_diagnostics::FlushDiagnostics;
pub(super) use super::flush_diagnostics::FlushFailureStage;
#[cfg(feature = "tracing-compat")]
use super::flush_diagnostics::trace_flush_caller_result;
#[cfg(all(test, not(loom)))]
use super::flush_measure::{FlushObserver, MetricsFlushObserver};
use super::flush_measure::{MeasuredOutcome, measure_flush_attempt, production_flush_observer};
use super::flush_metrics::FlushOutcome;
use super::flush_snapshot::clone_handler_snapshot;
#[cfg(all(test, not(loom)))]
pub(super) use super::flush_snapshot::flush_handler_snapshot_at;
pub(super) use super::flush_snapshot::flush_handler_snapshot_until;
use super::{FemtoLogger, LOGGER_FLUSH_TIMEOUT_MS};

// The `log` bridge is the only consumer, through the re-export in the parent
// module, so this follows that feature or `-D unused-imports` fails a lane with
// the bridge compiled out.
#[cfg(feature = "log-compat")]
pub(crate) use super::flush_measure::process_flush_metrics;
#[cfg(feature = "log-compat")]
pub(crate) use super::flush_measure::record_bridge_flush_failure;

/// Confirm a successful operation only when the supplied time is within its
/// absolute deadline.
pub(super) fn confirm_by_deadline(
    result: Result<(), FlushFailureStage>,
    deadline: Instant,
    now: Instant,
    late_stage: FlushFailureStage,
) -> Result<(), FlushFailureStage> {
    result.and_then(|()| {
        if now <= deadline {
            Ok(())
        } else {
            Err(late_stage)
        }
    })
}

/// Barrier result that retains its deadline across interpreter reattachment.
pub(super) struct BarrierOutcome {
    /// The result of marker insertion, worker acknowledgement, and handler flushes.
    result: Result<(), FlushFailureStage>,
    /// The one deadline consumed by every stage of this barrier, if representable.
    timing: FlushDeadline,
    /// Time at which Rust-side barrier work returned.
    barrier_completed_at: Instant,
}

impl BarrierOutcome {
    /// Record the final result after any required interpreter handoff.
    pub(super) fn finish(
        self,
        diagnostics: &FlushDiagnostics,
        late_stage: FlushFailureStage,
    ) -> bool {
        let now = self.timing.now();
        self.finish_at(now, diagnostics, late_stage)
    }

    /// Record the final result at an explicit time for generated tests.
    fn finish_at(
        self,
        now: Instant,
        diagnostics: &FlushDiagnostics,
        late_stage: FlushFailureStage,
    ) -> bool {
        let result = match self.timing.instant() {
            Some(deadline) => confirm_by_deadline(
                confirm_by_deadline(
                    self.result,
                    deadline,
                    self.barrier_completed_at,
                    FlushFailureStage::BarrierDeadlineExceeded,
                ),
                deadline,
                now,
                late_stage,
            ),
            None => Err(FlushFailureStage::DeadlineUnrepresentable),
        };
        let elapsed = now.saturating_duration_since(self.timing.started_at());
        // The caller's outcome is traced inside the barrier span so a late
        // decision, such as a Python return after the deadline, still belongs
        // to the flush that produced it.
        #[cfg(feature = "tracing-compat")]
        {
            let _entered = self.timing.span().enter();
            trace_flush_caller_result(result, elapsed);
        }
        diagnostics.record(result, elapsed);
        result.is_ok()
    }
}

impl MeasuredOutcome for BarrierOutcome {
    /// Collapse the barrier's fifteen failure stages onto the four counters.
    ///
    /// The metrics vocabulary is deliberately coarse: it answers "did this
    /// flush work, and if not, was it slow, was the shard gone, or did a
    /// handler refuse?" The stage itself stays in [`FlushDiagnostics`], which
    /// is where a specific cause is worth recording.
    fn metric_outcome(&self) -> FlushOutcome {
        match self.result {
            Ok(()) => FlushOutcome::Success,
            Err(
                FlushFailureStage::WorkerUnavailable
                | FlushFailureStage::MarkerEnqueueDisconnected
                | FlushFailureStage::WorkerStartDisconnected
                | FlushFailureStage::WorkerExited,
            ) => FlushOutcome::ChannelClosed,
            Err(
                FlushFailureStage::HandlerFlushUnconfirmed
                | FlushFailureStage::HandlerFlushDeadlineExceeded,
            ) => FlushOutcome::HandlerFailure,
            Err(
                FlushFailureStage::DeadlineUnrepresentable
                | FlushFailureStage::DispatchStateTimedOut
                | FlushFailureStage::HandlerSnapshotTimedOut
                | FlushFailureStage::DeadlineExpiredBeforeMarker
                | FlushFailureStage::MarkerEnqueueTimedOut
                | FlushFailureStage::WorkerStartTimedOut
                | FlushFailureStage::AcknowledgementTimedOut
                | FlushFailureStage::BarrierDeadlineExceeded
                | FlushFailureStage::CallerReturnDeadlineExceeded,
            ) => FlushOutcome::AckTimeout,
        }
    }
}

impl FemtoLogger {
    /// Flush all handlers attached to this logger.
    ///
    /// One absolute deadline spans dispatch resolution, the flush marker's
    /// admission, the handler snapshot, the worker's acknowledgement, and each
    /// handler's own flush, so the whole call is bounded once rather than once
    /// per stage. Built-in handlers keep their shorter per-handler timeouts.
    ///
    /// A handler that reports failure does not stop the ones after it: the
    /// sequence continues while budget remains, so one broken handler does not
    /// leave records buffered in the handlers after it. A failure that uses up
    /// the last of the budget ends the sequence, and the remaining handlers
    /// keep whatever they had buffered.
    ///
    /// When Python handlers can be attached, callers that currently hold
    /// Python access must detach before calling this method. Otherwise the
    /// worker cannot acquire the GIL to run a Python handler. A custom Rust
    /// handler must override [`FemtoHandlerTrait::flush_until`] to be
    /// confirmed, while a user-defined Python handler is confirmed through
    /// its own callable `flush`.
    ///
    /// # Returns
    ///
    /// `false` when the deadline expires before the barrier completes, when the
    /// shard is gone, or when any handler in the snapshot does not confirm its
    /// flush.
    ///
    /// # Examples
    ///
    /// A Rust caller flushes the logger's handlers:
    ///
    /// ```
    /// use _femtologging_rs::FemtoLogger;
    ///
    /// let logger = FemtoLogger::new(String::from("example"))
    ///     .expect("the shared dispatch pool starts");
    /// assert!(logger.flush_handlers());
    /// ```
    ///
    /// A caller that already holds Python access does not call this directly:
    /// it goes through the Python-facing entry points, which detach the
    /// interpreter for the whole barrier.
    pub fn flush_handlers(&self) -> bool {
        self.flush_handlers_blocking().finish(
            &self.flush_diagnostics,
            FlushFailureStage::CallerReturnDeadlineExceeded,
        )
    }

    /// Run the barrier and measure it once against the production boundary.
    ///
    /// The clock and the observer are chosen here, at the boundary: the
    /// orchestration below names neither the process-wide metrics nor the
    /// tracing feature. The observer opens its span when it is created, before
    /// the barrier, so an external subscriber sees the wait as part of the
    /// operation.
    pub(super) fn flush_handlers_blocking(&self) -> BarrierOutcome {
        let observer = production_flush_observer();
        measure_flush_attempt(&crate::monotonic_clock::SystemClock, &observer, || {
            self.flush_handlers_with_timing(FlushDeadline::system(Duration::from_millis(
                LOGGER_FLUSH_TIMEOUT_MS,
            )))
        })
    }

    /// Report any drops this logger has recorded but not yet warned about.
    ///
    /// Called at destruction only. The warners are rate limited, so a suppressed
    /// interval would otherwise leave the counts pending until a due moment that
    /// a logger about to disappear may never reach; reporting here makes the
    /// destruction mean "say what has been lost". An explicit flush cannot do
    /// this: it is deadline-bound, and the process-wide `log` sink invoked here
    /// writes synchronously, so a blocking sink would hold the caller past the
    /// deadline that bounds the whole call.
    pub(super) fn flush_drop_warners(&self) {
        self.drop_warner.flush(|count| {
            warn!("FemtoLogger: dropped {count} records; reported by flush");
        });
        #[cfg(feature = "python")]
        self.context_capture_warner.flush(|count| {
            warn!("FemtoLogger: dropped {count} records after context capture failed");
        });
    }

    /// Run the real barrier with a supplied clock for deterministic stage tests.
    #[cfg(all(test, not(loom)))]
    pub(super) fn flush_handlers_with_clock(
        &self,
        timeout: Duration,
        clock: SharedClock,
    ) -> BarrierOutcome {
        self.flush_handlers_with_timing(FlushDeadline::with_clock(timeout, clock))
    }

    /// Keep the single timing context alive through barrier and caller return.
    fn flush_handlers_with_timing(&self, timing: FlushDeadline) -> BarrierOutcome {
        let result = if timing.instant().is_some() {
            self.flush_barrier(&timing)
        } else {
            Err(FlushFailureStage::DeadlineUnrepresentable)
        };
        let barrier_completed_at = timing.now();
        // The logger-side warners are deliberately not flushed here: this path
        // is deadline-bound, and a synchronous sink can hold it past the whole
        // call's timeout. Those counts are reported at destruction, which has
        // no caller deadline to overrun.
        BarrierOutcome {
            result,
            timing,
            barrier_completed_at,
        }
    }

    /// Snapshot handlers and enqueue their flush after accepted dispatch work.
    fn flush_barrier(&self, timing: &FlushDeadline) -> Result<(), FlushFailureStage> {
        #[cfg(not(loom))]
        let (shard, budget) = match self.dispatch_state_until(timing) {
            Ok(Some(dispatch)) => dispatch,
            Ok(None) => return Err(FlushFailureStage::DispatchStateTimedOut),
            Err(_) => return Err(FlushFailureStage::WorkerUnavailable),
        };
        #[cfg(loom)]
        let (shard, budget) = (&*self.shard, &self.budget);

        #[cfg(not(loom))]
        let handler_guard = match self.handlers.try_read_for(timing.remaining()) {
            Some(guard) => guard,
            None => return Err(FlushFailureStage::HandlerSnapshotTimedOut),
        };
        #[cfg(loom)]
        let handler_guard = self.handlers.read();

        let has_handlers = !handler_guard.is_empty();
        drop(handler_guard);

        if timing.is_expired() {
            return Err(FlushFailureStage::DeadlineExpiredBeforeMarker);
        }

        if !has_handlers {
            return if budget.in_flight() > 0 {
                shard.wait_for_drain_until(timing)
            } else {
                Ok(())
            };
        }

        shard.flush_handlers_until(
            timing,
            |snapshot_sender| self.send_handler_snapshot_until(snapshot_sender, timing),
            || self.notify_barrier_waiting(),
        )
    }

    /// Announce that the barrier is now waiting on its worker.
    ///
    /// A Python regression test releases a blocked handler callback only once
    /// the flush reaches the point where the whole barrier waits on the
    /// worker. Announcing at barrier entry instead would let a flush that had
    /// merely started — still short of the wait, and free to stall on the
    /// registry or the queue before it — release that callback, so the test's
    /// in-flight assertion would no longer be about the wait itself. Both
    /// Python entry points reach the barrier, so the notification lives here
    /// rather than in `flush_handlers`.
    fn notify_barrier_waiting(&self) {
        #[cfg(all(feature = "python", feature = "test-util"))]
        self.flush_test_hooks.signal_entered();
    }

    /// Snapshot handlers only after marker admission, so a rejected marker has
    /// no handler owners to destroy on the flush caller.
    fn send_handler_snapshot_until(
        &self,
        snapshot_sender: Sender<Vec<Arc<dyn FemtoHandlerTrait>>>,
        timing: &FlushDeadline,
    ) -> Result<(), FlushFailureStage> {
        #[cfg(not(loom))]
        let handler_guard = match self.handlers.try_read_for(timing.remaining()) {
            Some(guard) => guard,
            None => return Err(FlushFailureStage::HandlerSnapshotTimedOut),
        };
        #[cfg(loom)]
        let handler_guard = self.handlers.read();

        let handlers = clone_handler_snapshot(&handler_guard);
        if timing.is_expired() {
            // The registry still owns every handler while this snapshot drops.
            drop(handlers);
            drop(handler_guard);
            return Err(FlushFailureStage::HandlerSnapshotTimedOut);
        }
        if let Err(error) = snapshot_sender.try_send(handlers) {
            // The registry still owns every handler if the worker has gone.
            drop(error);
            drop(handler_guard);
            return Err(FlushFailureStage::WorkerStartDisconnected);
        }
        drop(handler_guard);
        Ok(())
    }

    /// Run one real flush attempt with an injected clock and observer.
    ///
    /// Test-only: production selects the clock and the observer at the
    /// boundary in [`Self::flush_handlers_blocking`]. This entry point runs the
    /// same barrier, so a test can classify the shard and the attached
    /// handlers through the production path while the measurement lands in
    /// counters the test owns.
    #[cfg(all(test, not(loom)))]
    pub(crate) fn flush_handlers_measured<C, O>(&self, clock: &C, observer: &O) -> FlushOutcome
    where
        C: MonotonicClock + ?Sized,
        O: FlushObserver + ?Sized,
    {
        measure_flush_attempt(clock, observer, || {
            self.flush_handlers_with_timing(FlushDeadline::system(Duration::from_millis(
                LOGGER_FLUSH_TIMEOUT_MS,
            )))
        })
        .metric_outcome()
    }
}

#[cfg(all(test, not(loom)))]
#[path = "flush_tests.rs"]
mod tests;

#[cfg(all(test, not(loom)))]
#[path = "flush_barrier_tests.rs"]
mod barrier_tests;
