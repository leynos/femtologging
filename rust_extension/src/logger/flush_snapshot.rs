//! Flush a barrier's handler snapshot against its single deadline.
//!
//! A barrier captures its own handler owners and then flushes them in snapshot
//! order. Keeping the sequence here leaves [`super::flush`] responsible for the
//! shard handshake and the measurement boundary, while every caller — the shard
//! worker and the generated-time tests — runs the same loop.

use std::sync::Arc;

use crate::handler::{FemtoHandlerTrait, FlushContext, remaining};

use super::HandlerAttachment;
use super::flush::confirm_by_deadline;
use super::flush_clock::FlushDeadline;
use super::flush_diagnostics::FlushFailureStage;

/// Clone handler owners before releasing the registry lock.
pub(super) fn clone_handler_snapshot(
    handlers: &[HandlerAttachment],
) -> Vec<Arc<dyn FemtoHandlerTrait>> {
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
    context: &FlushContext,
) -> Result<(), FlushFailureStage> {
    flush_handler_sequence(handlers, context)
}

/// Flush a handler snapshot with the clock carried by its barrier context.
pub(super) fn flush_handler_snapshot_until(
    handlers: &[Arc<dyn FemtoHandlerTrait>],
    timing: &FlushDeadline,
) -> Result<(), FlushFailureStage> {
    if timing.instant().is_none() {
        return Err(FlushFailureStage::DeadlineUnrepresentable);
    }
    flush_handler_sequence(handlers, &timing.context())
}

/// Visit handlers in snapshot order, reporting the first unconfirmed flush.
///
/// Every handler receives the same context, so each wait in turn is charged
/// against one absolute deadline sampled from one clock. A flush is
/// best-effort: a handler that refuses must not leave the records buffered in
/// the handlers after it, so the sequence continues while budget remains. The
/// first failure is the one reported, because it is the one that explains the
/// flush the caller asked for.
fn flush_handler_sequence(
    handlers: &[Arc<dyn FemtoHandlerTrait>],
    context: &FlushContext,
) -> Result<(), FlushFailureStage> {
    let Some(deadline) = context.deadline() else {
        return Err(FlushFailureStage::DeadlineUnrepresentable);
    };
    let mut first_failure = None;
    for handler in handlers {
        if remaining(deadline, context.now()).is_zero() {
            first_failure.get_or_insert(FlushFailureStage::HandlerFlushDeadlineExceeded);
            break;
        }
        let confirmed = handler.flush_until(context);
        let completed_at = context.now();
        if !confirmed {
            first_failure.get_or_insert(FlushFailureStage::HandlerFlushUnconfirmed);
        } else if completed_at > deadline {
            first_failure.get_or_insert(FlushFailureStage::HandlerFlushDeadlineExceeded);
        }
    }
    match first_failure {
        Some(stage) => Err(stage),
        None => confirm_by_deadline(
            Ok(()),
            deadline,
            context.now(),
            FlushFailureStage::HandlerFlushDeadlineExceeded,
        ),
    }
}
