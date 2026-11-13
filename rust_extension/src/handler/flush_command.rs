//! The shared deadline and enqueue-then-acknowledge wait behind a handler
//! flush.
//!
//! Every handler that owns a worker queue performs the same two bounded waits:
//! admit a flush command, then wait for the worker's acknowledgement. Before
//! either wait it also has to turn the caller's deadline and its own configured
//! budget into one instant. Keeping both steps here lets the file, stream,
//! socket and HTTP handlers share one accounting rule — the budget spent on
//! admission is not offered again to the acknowledgement — while each keeps
//! ownership of its own channel type.

use std::time::{Duration, Instant};

use super::{capped_deadline, remaining};

/// The effective deadline a handler's own flush waits against.
///
/// A caller's deadline and the handler's configured budget are independent
/// bounds on the same flush, so the call stops at whichever arrives first.
/// The cap samples the clock once and hands the resulting instant to both
/// queue waits, which is what stops one flush spending the budget twice.
///
/// Handlers form the cap in their own order relative to preparing the queue:
/// the socket and HTTP handlers check their sender first, while the stream
/// handler prepares its sender afterwards, because that sender sits behind a
/// lock it must not hold while the deadline is sampled.
pub(crate) struct FlushCap<'a> {
    /// Absolute deadline supplied by the caller.
    caller_deadline: Instant,
    /// Budget this handler allows itself for one flush.
    budget: Duration,
    /// Monotonic source shared with the caller's context.
    now: &'a mut dyn FnMut() -> Instant,
}

impl<'a> FlushCap<'a> {
    /// Pair a caller's deadline with this handler's own budget.
    pub(crate) fn new(
        caller_deadline: Instant,
        budget: Duration,
        now: &'a mut dyn FnMut() -> Instant,
    ) -> Self {
        Self {
            caller_deadline,
            budget,
            now,
        }
    }

    /// Form the one instant both of a flush's queue waits stop at.
    ///
    /// The field is called rather than a method, so this expression always
    /// samples the caller's clock instead of recursing into the cap.
    pub(crate) fn effective(&mut self) -> Instant {
        capped_deadline(self.caller_deadline, (self.now)(), self.budget)
    }

    /// Borrow the clock so the flush waits can charge their time to it.
    pub(crate) fn clock(&mut self) -> &mut dyn FnMut() -> Instant {
        &mut *self.now
    }
}

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
