//! Deadline-aware handler snapshots and shard barriers for [`FemtoLogger`].
//!
//! The flush promise spans the handler snapshot, the dispatch shard's marker
//! and acknowledgement, and each handler's deadline-aware flush. This module
//! records a fixed-cardinality result after the barrier and, for Python calls,
//! after the interpreter has been reacquired.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pyo3::Python;

use crate::handler::{FemtoHandlerTrait, deadline_after, remaining};

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
    deadline: Option<Instant>,
    /// Time at which this call created its shared deadline.
    started_at: Instant,
    /// Time at which Rust-side barrier work returned.
    barrier_completed_at: Instant,
}

impl FlushOutcome {
    /// Record the final result after any required interpreter handoff.
    pub(super) fn finish(
        self,
        now: Instant,
        diagnostics: &FlushDiagnostics,
        late_stage: FlushFailureStage,
    ) -> bool {
        let result = match self.deadline {
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
        diagnostics.record(result, now.saturating_duration_since(self.started_at));
        result.is_ok()
    }
}

impl FemtoLogger {
    /// Flush every attached handler against one absolute deadline.
    pub(super) fn flush_handlers_blocking(&self) -> FlushOutcome {
        let started_at = Instant::now();
        let deadline = deadline_after(
            started_at,
            Duration::from_millis(LOGGER_FLUSH_TIMEOUT_MS),
        );
        let result = deadline.map_or(
            Err(FlushFailureStage::DeadlineUnrepresentable),
            |deadline| self.flush_barrier(deadline),
        );
        FlushOutcome {
            result,
            deadline,
            started_at,
            barrier_completed_at: Instant::now(),
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
            Instant::now(),
            &self.flush_diagnostics,
            FlushFailureStage::CallerReturnDeadlineExceeded,
        )
    }

    /// Snapshot handlers and enqueue their flush after accepted dispatch work.
    fn flush_barrier(&self, deadline: Instant) -> Result<(), FlushFailureStage> {
        #[cfg(not(loom))]
        let (shard, budget) = match self.dispatch_state_until(deadline) {
            Ok(Some(dispatch)) => dispatch,
            Ok(None) => return Err(FlushFailureStage::DispatchStateTimedOut),
            Err(_) => return Err(FlushFailureStage::WorkerUnavailable),
        };
        #[cfg(loom)]
        let (shard, budget) = (&*self.shard, &self.budget);

        #[cfg(not(loom))]
        let handler_guard = match self.handlers.try_read_for(remaining(deadline, Instant::now())) {
            Some(guard) => guard,
            None => return Err(FlushFailureStage::HandlerSnapshotTimedOut),
        };
        #[cfg(loom)]
        let handler_guard = self.handlers.read();

        let handlers = clone_handler_snapshot(&handler_guard);
        if remaining(deadline, Instant::now()).is_zero() {
            // Keep the registry's Arcs alive while discarding the clones. A
            // concurrent removal must not run a slow final destructor here.
            drop(handlers);
            drop(handler_guard);
            return Err(FlushFailureStage::DeadlineExpiredBeforeMarker);
        }

        if handlers.is_empty() {
            drop(handler_guard);
            if remaining(deadline, Instant::now()).is_zero() {
                return Err(FlushFailureStage::DeadlineExpiredBeforeMarker);
            }
            return if budget.in_flight() > 0 {
                shard.wait_for_drain_until(deadline)
            } else {
                Ok(())
            };
        }

        shard.flush_handlers_until(deadline, handlers, handler_guard)
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
pub(super) fn flush_handler_snapshot_at(
    handlers: &[Arc<dyn FemtoHandlerTrait>],
    deadline: Instant,
    now: impl FnMut() -> Instant,
) -> Result<(), FlushFailureStage> {
    flush_handler_sequence(handlers, deadline, now)
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
