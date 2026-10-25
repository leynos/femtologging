//! The two in-flight bounds a shard puts on queued work.
//!
//! The queue depth bounds memory: a burst cannot grow a shard's backlog past
//! its channel capacity. That alone does not bound *latency*, though, because a
//! logger whose handler never returns keeps its records queued forever. So a
//! shard also hands out a fixed number of permits, one per record it will hold
//! in flight, and a producer takes one before it queues. A permit travels
//! inside the job it authorizes and returns to the pool from its own `Drop`,
//! which is what makes the accounting survive a panicking handler.
//!
//! A shard-wide permit pool is still not enough to make a shard *shared*: one
//! logger could take every permit and starve the others. Each logger therefore
//! has its own in-flight budget within the pool, charged by the logging thread
//! at admission and released by the same permit. That is the per-logger bound
//! ADR 007 promises.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::sync::{Receiver, Sender, bounded};

/// The permits bounding how much work one shard holds.
pub(super) struct Permits {
    /// One token per free in-flight slot, taken non-blockingly by a producer.
    pub(super) available: Receiver<()>,
    /// Where a used token goes back.
    ///
    /// Kept alongside `available` so the channel's senders are never all
    /// dropped: a disconnected channel is not an exhausted pool, and the two
    /// would otherwise be indistinguishable to `try_recv`.
    pub(super) returned: Sender<()>,
}

/// Permission to hold one record in flight on a shard.
///
/// It travels to the worker inside the job it authorizes, and returns to the
/// pool when that job is dropped — after the handlers have run, or while a
/// panicking handler unwinds. A permit therefore counts its record as in flight
/// for exactly as long as the record is unhandled, on every path out.
///
/// The permit also carries the logger's own in-flight budget, and releases it
/// through the same `Drop`. Both halves are acquired together and released
/// together, so the two counts cannot drift apart.
pub(super) struct Permit {
    /// The shard-wide token pool this permit returns to when dropped.
    permits: Arc<Permits>,
    /// The logger whose record this permit authorizes.
    logger: Arc<LoggerBudget>,
    /// Process that charged the logger budget, so an inherited permit cannot
    /// release a charge after the child resets that budget.
    #[cfg(not(loom))]
    process_id: u32,
}

/// One logger's share of a shard's in-flight capacity.
///
/// The shard-wide permit pool alone bounds what a shard holds, not what one
/// logger consumes: a single logger whose handler never returns would take
/// every permit and starve every other logger on its shard, which makes the
/// shared pool a queue for whoever arrived first rather than a shared
/// resource. This budget is charged by the logging thread in
/// [`Shard::admit`](super::Shard::admit), so it bounds a logger's backlog at
/// the point it is created, and it is released by [`Permit`]'s drop, after the
/// handlers have run.
///
/// This is the per-logger in-flight bound ADR 007 promises. It is created by
/// [`Shard::budget`](super::Shard::budget) and owned by the logger.
pub(crate) struct LoggerBudget {
    /// How many of this logger's records may be unhandled at once.
    limit: usize,
    /// Records logged through this logger that the shard has not finished.
    in_flight: AtomicUsize,
}

impl Permits {
    /// Build a pool holding `capacity` free permits.
    ///
    /// Every send fills one of the channel's own slots, so the loop cannot
    /// block. The result is discarded rather than expected: the only way to
    /// lose a token is for the channel to be full or gone, and in both cases
    /// the pool ends up smaller than asked for — it refuses more work, never
    /// accepts more — so degrading quietly is the safe direction. Panicking
    /// instead would take down a process that is merely short of a resource.
    pub(super) fn new(capacity: usize) -> Self {
        let (returned, available) = bounded::<()>(capacity);
        for _ in 0..capacity {
            let _ = returned.send(());
        }
        Self {
            available,
            returned,
        }
    }
}

impl LoggerBudget {
    /// Create a budget for one logger, holding nothing in flight.
    ///
    /// The limit is chosen by [`Shard::budget`](super::Shard::budget): half the
    /// shard's depth, never more than
    /// [`RECORDS_PER_LOGGER`](super::super::RECORDS_PER_LOGGER). It is always
    /// below the depth, so a logger cannot take every permit. The parameter
    /// also lets a test size a budget small enough to reach in a shard it can
    /// afford to build.
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            in_flight: AtomicUsize::new(0),
        }
    }

    /// Forget charges held by workers that did not survive a fork.
    ///
    /// A child inherits the logger's atomic state but not the parent's worker
    /// threads. Its replacement shard starts with an empty budget, and a permit
    /// inherited from the parent cannot release a charge from it.
    #[cfg(not(loom))]
    pub(crate) fn reset_after_fork(&self) {
        self.in_flight.store(0, Ordering::Release);
    }

    /// Return the number of unhandled records this logger may hold.
    fn limit(&self) -> usize {
        self.limit
    }

    /// Return the limit a test can compare against a shard's depth.
    ///
    /// The invariant that matters — a budget is always *below* its shard's
    /// depth — is arithmetic, so it is asserted directly rather than by
    /// building a shard deep enough to reach a production-sized limit.
    #[cfg(test)]
    pub(crate) fn limit_for_test(&self) -> usize {
        self.limit()
    }

    /// Return how many of this logger's records the shard has not finished.
    ///
    /// Read by `Drop` to decide whether waiting for the shard could possibly
    /// help: a logger with nothing in flight has nothing of its own to drain.
    ///
    /// Gated out under Loom like its only caller: a model's shard is the
    /// logger's own, so its drop shuts the shard down and joins the worker
    /// rather than waiting on a shared barrier, and nothing reads the count.
    #[cfg(not(loom))]
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }

    /// Charge one record against the logger's budget, or refuse it.
    ///
    /// Takes no lock and never waits. A logger at its budget is already holding
    /// as much unhandled work as it is allowed, so refusing here is what stops
    /// one logger's slow handler becoming every other logger's outage.
    fn try_charge(&self) -> bool {
        // A CAS loop rather than `fetch_add` then a rollback, so a refusal
        // never temporarily inflates the count another thread reads. Released
        // with `AcqRel` for the same reason: `in_flight` is read by `Drop` to
        // decide whether to wait, and it must see a charge that has been
        // released.
        self.in_flight
            .try_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < self.limit()).then_some(held + 1)
            })
            .is_ok()
    }

    /// Return one charge to the budget.
    ///
    /// Called from [`Permit`]'s drop, so a handler panic unwinds through it and
    /// the charge is still returned.
    fn release(&self) {
        // Saturating, because a release without a matching charge would
        // otherwise underflow to a figure no bound can compare against.
        let _ = self
            .in_flight
            .try_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                held.checked_sub(1)
            });
    }
}

impl Permit {
    /// Take a permit if the shard and the logger both have room.
    ///
    /// The logger's budget is charged first: it is the tighter bound, and
    /// checking it costs no channel operation. Only then is a shard-wide token
    /// taken, so a logger refused by its own budget never disturbs the pool.
    /// Never waits: a shard that has handed out every permit is already holding
    /// as much work as it will, so the caller counts a dropped record rather
    /// than blocking the logging call.
    pub(super) fn take(permits: &Arc<Permits>, logger: &Arc<LoggerBudget>) -> Option<Self> {
        if !logger.try_charge() {
            return None;
        }
        match permits.available.try_recv() {
            Ok(()) => Some(Self {
                permits: Arc::clone(permits),
                logger: Arc::clone(logger),
                #[cfg(not(loom))]
                process_id: std::process::id(),
            }),
            // The shard is full, so nothing was queued. Undo the budget charge
            // here rather than letting it leak: a charge with no permit would
            // shrink the logger's budget a little more with every refused
            // record, and the logger would eventually refuse every record.
            Err(_) => {
                logger.release();
                None
            }
        }
    }
}

impl Drop for Permit {
    /// Return the permit to its pool and the charge to its logger's budget.
    ///
    /// Releasing from `Drop` rather than a named method is what makes the
    /// accounting hold on every path out of a job, including one where a
    /// handler panics and unwinds through it.
    fn drop(&mut self) {
        // A failed send can only mean the pool is already full, which would be
        // a double release; ignoring it keeps the count from inflating.
        let _ = self.permits.returned.try_send(());
        #[cfg(not(loom))]
        if self.process_id != std::process::id() {
            return;
        }
        self.logger.release();
    }
}
