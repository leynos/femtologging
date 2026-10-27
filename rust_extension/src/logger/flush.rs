//! Deadline-aware handler snapshots and shard barriers for [`FemtoLogger`].
//!
//! The flush promise spans the handler snapshot, the dispatch shard's marker
//! and acknowledgement, and each handler's deadline-aware flush. This module
//! records a fixed-cardinality result after the barrier and, for Python calls,
//! after the interpreter has been reacquired.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::sync::Sender;
use pyo3::Python;

use crate::handler::{FemtoHandlerTrait, remaining};

use super::flush_clock::FlushDeadline;
#[cfg(all(test, not(loom)))]
use super::flush_clock::MonotonicClock;
use super::flush_diagnostics::FlushDiagnostics;
pub(super) use super::flush_diagnostics::FlushFailureStage;
use super::{FemtoLogger, HandlerAttachment, LOGGER_FLUSH_TIMEOUT_MS};

/// Confirm a successful operation only when the supplied time is within its
/// absolute deadline.
fn confirm_by_deadline(
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
pub(super) struct FlushOutcome {
    /// The result of marker insertion, worker acknowledgement, and handler flushes.
    result: Result<(), FlushFailureStage>,
    /// The one deadline consumed by every stage of this barrier, if representable.
    timing: FlushDeadline,
    /// Time at which Rust-side barrier work returned.
    barrier_completed_at: Instant,
}

impl FlushOutcome {
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
        diagnostics.record(
            result,
            now.saturating_duration_since(self.timing.started_at()),
        );
        result.is_ok()
    }
}

impl FemtoLogger {
    /// Flush every attached handler against one absolute deadline.
    pub(super) fn flush_handlers_blocking(&self) -> FlushOutcome {
        let timing = FlushDeadline::system(Duration::from_millis(LOGGER_FLUSH_TIMEOUT_MS));
        self.flush_handlers_with_timing(timing)
    }

    /// Run the real barrier with a supplied clock for deterministic stage tests.
    #[cfg(all(test, not(loom)))]
    fn flush_handlers_with_clock(
        &self,
        timeout: Duration,
        clock: Arc<dyn MonotonicClock>,
    ) -> FlushOutcome {
        self.flush_handlers_with_timing(FlushDeadline::with_clock(timeout, clock))
    }

    /// Keep the single timing context alive through barrier and caller return.
    fn flush_handlers_with_timing(&self, timing: FlushDeadline) -> FlushOutcome {
        let result = if timing.instant().is_some() {
            self.flush_barrier(&timing)
        } else {
            Err(FlushFailureStage::DeadlineUnrepresentable)
        };
        let barrier_completed_at = timing.now();
        FlushOutcome {
            result,
            timing,
            barrier_completed_at,
        }
    }

    /// Release Python while the shard and handlers consume the flush deadline.
    pub(crate) fn flush_handlers_detached(&self, py: Python<'_>) -> bool {
        #[cfg(not(loom))]
        let outcome = py.detach(|| self.flush_handlers_blocking());
        #[cfg(loom)]
        let outcome = {
            let _ = py;
            self.flush_handlers_blocking()
        };
        outcome.finish(
            &self.flush_diagnostics,
            FlushFailureStage::CallerReturnDeadlineExceeded,
        )
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
            if timing.is_expired() {
                return Err(FlushFailureStage::DeadlineExpiredBeforeMarker);
            }
            return if budget.in_flight() > 0 {
                shard.wait_for_drain_until(timing)
            } else {
                Ok(())
            };
        }

        shard.flush_handlers_until(timing, |snapshot_sender| {
            self.send_handler_snapshot_until(snapshot_sender, timing)
        })
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
}

/// Clone handler owners before releasing the registry lock.
fn clone_handler_snapshot(handlers: &[HandlerAttachment]) -> Vec<Arc<dyn FemtoHandlerTrait>> {
    handlers
        .iter()
        .map(|attachment| Arc::clone(&attachment.handler))
        .collect()
}

/// Flush a barrier's handler snapshot using the same timing logic in runtime
/// and generated-time tests.
#[cfg(all(test, not(loom)))]
pub(super) fn flush_handler_snapshot_at(
    handlers: &[Arc<dyn FemtoHandlerTrait>],
    deadline: Instant,
    now: impl FnMut() -> Instant,
) -> Result<(), FlushFailureStage> {
    flush_handler_sequence(handlers, deadline, now)
}

/// Flush a handler snapshot with the clock carried by its barrier context.
pub(super) fn flush_handler_snapshot_until(
    handlers: &[Arc<dyn FemtoHandlerTrait>],
    timing: &FlushDeadline,
) -> Result<(), FlushFailureStage> {
    let Some(deadline) = timing.instant() else {
        return Err(FlushFailureStage::DeadlineUnrepresentable);
    };
    flush_handler_sequence(handlers, deadline, || timing.now())
}

/// Visit handlers in snapshot order and stop at the first unconfirmed flush.
fn flush_handler_sequence(
    handlers: &[Arc<dyn FemtoHandlerTrait>],
    deadline: Instant,
    mut now: impl FnMut() -> Instant,
) -> Result<(), FlushFailureStage> {
    for handler in handlers {
        if remaining(deadline, now()).is_zero() {
            return Err(FlushFailureStage::HandlerFlushDeadlineExceeded);
        }
        let confirmed = handler.flush_until(deadline);
        let completed_at = now();
        if !confirmed {
            return Err(FlushFailureStage::HandlerFlushUnconfirmed);
        }
        if completed_at > deadline {
            return Err(FlushFailureStage::HandlerFlushDeadlineExceeded);
        }
    }
    confirm_by_deadline(
        Ok(()),
        deadline,
        now(),
        FlushFailureStage::HandlerFlushDeadlineExceeded,
    )
}

#[cfg(all(test, not(loom)))]
#[path = "flush_tests.rs"]
mod tests;
