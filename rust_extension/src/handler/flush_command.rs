//! The shared enqueue-then-acknowledge wait behind a handler flush.
//!
//! Every handler that owns a worker queue performs the same two bounded waits:
//! admit a flush command, then wait for the worker's acknowledgement. Keeping
//! the wait here lets the file, stream, socket and HTTP handlers share one
//! accounting rule — the budget spent on admission is not offered again to the
//! acknowledgement — while each keeps ownership of its own channel type.

use std::time::{Duration, Instant};

use super::remaining;

/// Which of a flush's two queue waits ran out of budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QueueFlushFailure {
    /// The bounded command queue did not accept the flush before the deadline.
    Enqueue,
    /// The worker accepted the flush but did not acknowledge it before the deadline.
    Acknowledgement,
}

/// Hand a flush command to a worker queue and wait for its acknowledgement.
///
/// `enqueue` receives the budget left when admission begins and returns, once
/// the command is accepted, a closure that awaits the acknowledgement. It
/// returns `None` when the bounded queue refused the command. Both budgets are
/// sampled through `now`, so waiting for queue room is charged once: a marker
/// that spent most of the deadline being admitted leaves the rest, not a fresh
/// copy, for the acknowledgement. Production passes [`Instant::now`] at this
/// boundary; tests pass a scripted source so the budget handed to each wait
/// can be shown to shrink deterministically, even with a stalled worker.
///
/// The caller owns the channel because not every handler queue is the same
/// type: the file, stream and HTTP handlers use the crate's `sync` seam, while
/// the socket handler's worker owns its own crossbeam channel.
///
/// # Errors
///
/// Returns [`QueueFlushFailure::Enqueue`] when the bounded queue stays full for
/// the whole budget, and [`QueueFlushFailure::Acknowledgement`] when the
/// worker accepts the command but does not answer in time.
pub(crate) fn flush_command_until<A, Ack>(
    deadline: Instant,
    now: &mut dyn FnMut() -> Instant,
    enqueue: impl FnOnce(Duration) -> Option<Ack>,
) -> Result<A, QueueFlushFailure>
where
    Ack: FnOnce(Duration) -> Option<A>,
{
    let Some(await_acknowledgement) = enqueue(remaining(deadline, now())) else {
        return Err(QueueFlushFailure::Enqueue);
    };
    await_acknowledgement(remaining(deadline, now())).ok_or(QueueFlushFailure::Acknowledgement)
}
