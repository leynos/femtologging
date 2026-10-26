//! Deadline-aware handler snapshots and shard barriers for [`FemtoLogger`].
//!
//! The flush promise spans the handler snapshot, the dispatch shard's marker
//! and acknowledgement, and each handler's own deadline-aware flush. Keeping
//! this path here makes it harder to give one stage a fresh timeout or retain
//! the handler registry lock while user code runs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pyo3::Python;

use crate::handler::{FemtoHandlerTrait, remaining};

use super::{FemtoLogger, HandlerAttachment, LOGGER_FLUSH_TIMEOUT_MS};

/// Barrier result that retains its deadline across interpreter reattachment.
pub(super) struct FlushOutcome {
    /// Whether the shard and all opted-in handlers confirmed the barrier.
    completed: bool,
    /// The one deadline consumed by every stage of this barrier.
    deadline: Instant,
}

impl FlushOutcome {
    /// Confirm success only when the Rust barrier and interpreter handoff fit.
    pub(super) fn is_confirmed(&self) -> bool {
        self.completed && Instant::now() <= self.deadline
    }
}

impl FemtoLogger {
    /// Flush every attached handler against one absolute deadline.
    pub(super) fn flush_handlers_blocking(&self) -> FlushOutcome {
        let deadline = Instant::now() + Duration::from_millis(LOGGER_FLUSH_TIMEOUT_MS);
        FlushOutcome {
            completed: self.flush_barrier(deadline) && Instant::now() <= deadline,
            deadline,
        }
    }

    /// Release Python while the shard and handlers consume the flush deadline.
    pub(crate) fn flush_handlers_detached(&self, py: Python<'_>) -> bool {
        #[cfg(not(loom))]
        {
            py.detach(|| self.flush_handlers_blocking()).is_confirmed()
        }
        #[cfg(loom)]
        {
            let _ = py;
            self.flush_handlers_blocking().is_confirmed()
        }
    }

    /// Snapshot handlers and enqueue their flush after accepted dispatch work.
    fn flush_barrier(&self, deadline: Instant) -> bool {
        #[cfg(not(loom))]
        let (shard, budget) = match self.dispatch_state() {
            Ok(dispatch) => dispatch,
            Err(_) => return false,
        };
        #[cfg(not(loom))]
        let handler_guard = match self.handlers.try_read_for(remaining(deadline)) {
            Some(guard) => guard,
            None => return false,
        };
        #[cfg(loom)]
        let handler_guard = self.handlers.read();

        let handlers = clone_handler_snapshot(&handler_guard);
        if remaining(deadline).is_zero() {
            // Keep the registry's Arcs alive while discarding the clones. A
            // concurrent removal must not run a slow final destructor here.
            drop(handlers);
            drop(handler_guard);
            return false;
        }

        if handlers.is_empty() {
            drop(handler_guard);
            if Instant::now() > deadline {
                return false;
            }
            #[cfg(not(loom))]
            let has_in_flight_work = budget.in_flight() > 0;
            #[cfg(loom)]
            let has_in_flight_work = self.budget.in_flight() > 0;
            return if has_in_flight_work {
                #[cfg(not(loom))]
                {
                    shard.wait_for_drain_until(deadline)
                }
                #[cfg(loom)]
                {
                    self.shard.wait_for_drain_until(deadline)
                }
            } else {
                true
            };
        }

        #[cfg(not(loom))]
        return shard.flush_handlers_until(deadline, handlers, handler_guard);
        #[cfg(loom)]
        self.shard
            .flush_handlers_until(deadline, handlers, handler_guard)
    }
}

/// Clone handler owners before releasing the registry lock.
fn clone_handler_snapshot(handlers: &[HandlerAttachment]) -> Vec<Arc<dyn FemtoHandlerTrait>> {
    handlers
        .iter()
        .map(|attachment| Arc::clone(&attachment.handler))
        .collect()
}
