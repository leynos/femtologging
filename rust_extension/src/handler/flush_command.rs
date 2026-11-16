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

/// How a handler hands its due drop count to a flush marker, and takes it back.
///
/// The pair travels together because either half alone is unusable: the take is
/// destructive, so a flush that cannot enqueue must undo it. Keeping them in one
/// value also keeps [`flush_handler_until`]'s argument list to the four things it
/// genuinely needs.
pub(crate) struct PendingDrops<T, R> {
    /// Supplies the count that is due, removing it from the rate limiter.
    take: T,
    /// Puts a count back when the marker never entered the queue.
    restore: R,
}

impl<T, R> PendingDrops<T, R>
where
    T: FnOnce() -> Option<u64>,
    R: FnOnce(u64),
{
    /// Pair a destructive take with the restore that undoes it.
    ///
    /// Example: `PendingDrops::new(|| warner.take_due(), |n| warner.restore_drops(n))`.
    pub(crate) fn new(take: T, restore: R) -> Self {
        Self { take, restore }
    }
}

/// Run one handler flush against a capped deadline, carrying its due drops.
///
/// The handler's own budget caps the caller's deadline first, then `prepare`
/// supplies the queue to write to; a handler whose worker has gone returns
/// `None` without waiting. The pending drop count travels on the marker so the
/// logging callback runs on the worker rather than on the flush caller, and it
/// goes back through the restore half of [`PendingDrops`] when the marker never
/// entered the queue — a saturated or closed queue would otherwise strand the
/// records the flush was meant to report.
///
/// The count and its restore arrive as a value rather than as a warner
/// reference. Rate-limit policy decides *which* drops are pending; this helper
/// decides *where* they travel. Keeping the two apart is what lets the stream,
/// HTTP and socket handlers share one transport without sharing a policy type.
///
/// The take runs only once a queue was found, because taking the count is
/// destructive: a handler whose worker has gone must leave the drops pending for
/// whoever flushes next.
///
/// `prepare` runs after the cap deliberately. The stream handler's sender sits
/// behind a mutex, and sampling the deadline while holding it would make one
/// flush wait on another thread's flush.
pub(crate) fn flush_handler_until<S, A, Ack>(
    pending: PendingDrops<impl FnOnce() -> Option<u64>, impl FnOnce(u64)>,
    cap: &mut FlushCap<'_>,
    prepare: impl FnOnce() -> Option<S>,
    enqueue: impl FnOnce(S, Option<u64>, Duration) -> Option<Ack>,
) -> Option<Result<A, QueueFlushFailure>>
where
    Ack: FnOnce(Duration) -> Option<A>,
{
    let PendingDrops { take, restore } = pending;
    let effective = cap.effective();
    let sender = prepare()?;
    let pending_drops = take();
    let outcome = flush_command_until(effective, cap.clock(), |budget| {
        enqueue(sender, pending_drops, budget)
    });
    if matches!(outcome, Err(QueueFlushFailure::Enqueue))
        && let Some(count) = pending_drops
    {
        restore(count);
    }
    Some(outcome)
}

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
/// worker accepts the command but does not answer before the deadline. The
/// second case includes an answer that is *received* after the deadline: a
/// zero-budget receive can still take a message that was already queued, and a
/// descheduled waiter can observe it late, so the clock is sampled once more
/// before an acknowledgement is reported as success.
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
    let acknowledged = await_acknowledgement(remaining(deadline, now()))
        .ok_or(QueueFlushFailure::Acknowledgement)?;
    // A handler's flush contract promises not to report success after its
    // deadline. The receive above enforces that only as far as the channel
    // does: a ready message wins over an expired budget, so the answer itself
    // has to be checked against the clock before it counts.
    if now() > deadline {
        return Err(QueueFlushFailure::Acknowledgement);
    }
    Ok(acknowledged)
}
