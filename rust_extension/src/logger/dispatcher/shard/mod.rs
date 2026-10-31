//! One shard's queue, its in-flight permits, and the worker that drains it.
//!
//! The queue carries two kinds of job and knows nothing about handlers:
//! [`Job::Record`] hands a queued record to the handler set the logger captured
//! with it, and [`Job::Barrier`] asks for an acknowledgement once everything
//! queued ahead of it has been handled. Sharing the queue is what lets a
//! logger's flush travel the same path as its records, so an acknowledgement
//! cannot overtake one.
//!
//! Work is bounded three times, and the third bound is the one that keeps a
//! shard shared rather than merely multiplexed. The queue is bounded, so a
//! burst cannot grow memory without limit. A shard-wide permit pool bounds how
//! much work it holds *in flight*, so a slow handler cannot turn a busy
//! logger's backlog into an unbounded one. And each logger has its own
//! in-flight budget within that pool, so one logger whose handler never
//! returns cannot consume every permit and starve the loggers sharing its
//! shard. All three refusals are returned to the caller, which counts the drop
//! and warns about it: the logging call itself never waits.
//!
//! The two in-flight bounds live in [`budget`] and the drain loop in
//! [`worker`]; this module defines the job type, the shard handle, and the
//! admission and barrier operations a producer calls.

mod budget;
mod worker;

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::logger::QueuedRecord;
#[cfg(loom)]
use crate::sync::{JoinHandle, Mutex};
use crate::sync::{Sender, bounded, recv_timeout};

pub(crate) use budget::LoggerBudget;
use budget::{Permit, Permits};
use worker::run_worker;

/// How long a barrier waits for the shard to acknowledge it.
///
/// Bounded because a barrier is waited on from `Drop` and from
/// `flush_handlers`: a handler that never returns must delay those calls, not
/// hang them. A timed-out barrier loses the queued records, which is what the
/// per-logger worker did when its own flush timed out. The bound is ignored
/// under Loom, so no model depends on it firing.
const BARRIER_TIMEOUT: Duration = Duration::from_millis(2_000);

/// A job for a shard.
enum Job {
    /// Handle `queued` with the handlers it carries, holding `permit` while it
    /// is in flight.
    ///
    /// Boxed because the queue's buffer is allocated at its full depth: an
    /// inline record would make every slot of every shard carry the record's
    /// several hundred bytes whether or not a record ever occupies it.
    Record {
        /// The record and handler snapshot awaiting the worker.
        queued: Box<QueuedRecord>,
        /// The shard and logger capacity held until the record is handled.
        permit: Permit,
    },
    /// Signal `ack` once every job ahead of this one has been handled.
    Barrier {
        /// The channel used to acknowledge the completed barrier.
        ack: Sender<()>,
    },
    /// Finish the jobs already queued and stop the worker.
    ///
    /// Only a caller that owns the shard sends this: a process-wide pool shard
    /// has no owner and its worker lives until the process ends.
    #[cfg(loom)]
    Stop,
}

/// One shard's backlog, and the permits bounding what it holds.
pub(crate) struct Shard {
    /// Queue used by producers to submit records and drain barriers.
    queue: Sender<Job>,
    /// One free token per slot of in-flight work.
    permits: Arc<Permits>,
    /// How many records this shard will hold at once.
    ///
    /// Kept so a logger's budget can be sized *below* it. A budget equal to the
    /// depth would let one logger take every permit on a shallow shard, which
    /// is exactly the starvation the budget exists to prevent.
    depth: usize,
    /// The shard's worker, under Loom only.
    ///
    /// A process shares one pool of shards that lives as long as the process,
    /// so there is no handle to keep and a joining thread would have nothing to
    /// wait for: every worker stays until its queue is dropped, which happens
    /// at process exit. A Loom model builds its own shard for the run and must
    /// join that model thread before the run ends.
    #[cfg(loom)]
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Shard {
    /// Start a shard that holds at most `depth` records in flight.
    ///
    /// # Errors
    ///
    /// Returns the operating system's refusal when the shard's worker thread
    /// cannot be started.
    pub(crate) fn start(index: usize, depth: usize) -> Result<Self, std::io::Error> {
        let (queue, jobs) = bounded::<Job>(depth);
        let permits = Arc::new(Permits::new(depth));
        // The worker is handed the queue alone: the permits travel to it
        // inside the jobs, and the shard keeps the pool they belong to alive
        // for as long as the worker can be running.
        let _worker =
            crate::sync::spawn_named(format!("femtologging-dispatch-{index}"), move || {
                run_worker(jobs);
            })?;
        Ok(Self {
            queue,
            permits,
            depth,
            #[cfg(loom)]
            worker: Mutex::new(Some(_worker)),
        })
    }

    /// Create the in-flight budget a logger assigned to this shard draws from.
    ///
    /// One budget per logger, not per shard: it is what stops one logger's slow
    /// handler consuming the whole shard.
    ///
    /// The limit is capped *below* the shard's depth, because a budget equal to
    /// it would be no bound at all. A single-core machine, or a cgroup quota of
    /// one CPU, gives a shard count of one, and then the pool's depth is
    /// [`RECORDS_PER_LOGGER`](super::RECORDS_PER_LOGGER) exactly — the figure
    /// ADR 0007 names as one logger's share. Granting it in full would let one
    /// logger take every permit and every other logger on that shard be
    /// refused, which is the starvation this budget exists to prevent.
    ///
    /// Half the depth, so a shard can always serve at least two loggers, and
    /// never zero: a shard too shallow to halve still admits one record at a
    /// time rather than none.
    pub(crate) fn budget(&self) -> Arc<LoggerBudget> {
        // `max(1)` first: a one-deep shard halves to zero, and a budget of zero
        // would refuse every record on a shard that can still hold one.
        let share = (self.depth / 2).max(1);
        // Then the documented ceiling, which a deep pool shard reaches: the
        // depth is `RECORDS_PER_LOGGER * count`, so half of it exceeds the
        // per-logger figure whenever the pool has more than two shards.
        Arc::new(LoggerBudget::new(share.min(super::RECORDS_PER_LOGGER)))
    }

    /// Create a budget with an explicit limit, for a test.
    ///
    /// The production limit is a thousand times larger than a test shard can
    /// hold, so a test cannot reach it without either a thousand-deep shard or
    /// a thousand blocked handlers. Sizing the budget directly lets a shallow
    /// shard exercise the same refusal: the property under test is that the
    /// shard-wide pool and a logger's own allowance are separate bounds, which
    /// does not depend on either number's magnitude.
    #[cfg(all(test, not(loom)))]
    pub(crate) fn budget_with_limit_for_test(&self, limit: usize) -> Arc<LoggerBudget> {
        Arc::new(LoggerBudget::new(limit))
    }

    /// Accept a record for dispatch, without waiting.
    ///
    /// The record is taken and returned boxed, which is the shape the queue
    /// needs: every slot of a shard's buffer is allocated at its full depth, so
    /// an inline record would put several hundred bytes in each one whether or
    /// not it is ever used. Boxing at the call site keeps that allocation to
    /// exactly one per record, and the returned error is then a pointer rather
    /// than a copy of the record.
    ///
    /// # Errors
    ///
    /// Returns the record back, still boxed and untouched, when the shard is
    /// full or gone, or when `budget` says this logger already holds its
    /// allowance. Either way the caller can count the drop and warn about it
    /// without paying for an allocation on the path that runs when the process
    /// is already short of a resource.
    pub(crate) fn admit(
        &self,
        queued: Box<QueuedRecord>,
        budget: &Arc<LoggerBudget>,
    ) -> Result<(), Box<QueuedRecord>> {
        let Some(permit) = Permit::take(&self.permits, budget) else {
            return Err(queued);
        };
        match self.queue.try_send(Job::Record { queued, permit }) {
            Ok(()) => Ok(()),
            // The queue filled or went away after a permit was taken. The
            // permit travelled in the rejected job, so unpacking it drops the
            // guard and returns the token; the record goes back to the caller.
            Err(error) => match error.into_inner() {
                Job::Record { queued, .. } => Err(queued),
                Job::Barrier { .. } => unreachable!("only a record job is admitted here"),
                // `admit` sends nothing but records, so neither of these can
                // come back. The arm exists only because a `--cfg loom` build
                // adds `Job::Stop`, and a match over the full enum must name
                // it.
                #[cfg(loom)]
                Job::Stop => unreachable!("only a record job is admitted here"),
            },
        }
    }

    /// Wait for the shard to finish the work queued ahead of this call.
    ///
    /// Returns `false` when the shard is gone or does not answer within
    /// [`BARRIER_TIMEOUT`], which the caller reports as a failed flush or an
    /// incomplete drain.
    ///
    /// The barrier is queued with `send_timeout` rather than `try_send`: a
    /// shard at the end of a burst has a full queue but a working worker, and
    /// refusing the barrier then would report a drain that did happen as a
    /// flush that failed. Waiting for room makes the answer mean "everything
    /// ahead of this call was handled", which is what a flush promises.
    ///
    /// Both waits share one deadline, so a call is bounded by
    /// [`BARRIER_TIMEOUT`] in total rather than by twice it. Queueing the
    /// barrier and hearing its answer are two halves of one wait, and a
    /// caller that was promised a two-second drain would otherwise block for
    /// four when the queue is full and the worker is slow.
    pub(crate) fn wait_for_drain(&self) -> bool {
        let deadline = Instant::now() + BARRIER_TIMEOUT;
        let (ack, acked) = bounded::<()>(1);
        if self
            .queue
            .send_timeout(Job::Barrier { ack }, BARRIER_TIMEOUT)
            .is_err()
        {
            return false;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        recv_timeout(&acked, remaining).is_ok()
    }

    /// Finish the queued work and join the worker, which only a Loom run has.
    ///
    /// A Loom model must not end with a live model thread, so the run that
    /// started the shard is the run that ends it. The stop travels the queue,
    /// so the records queued ahead of it are handled first.
    ///
    /// The stop is sent blocking rather than with `try_send`: a shard whose
    /// queue happens to be full must still be told to finish, or the join
    /// below would wait for a worker that was never asked to stop. Blocking is
    /// safe here because the worker is draining that same queue, so room
    /// arrives as fast as the worker can handle a job.
    #[cfg(loom)]
    pub(crate) fn shut_down(&self) {
        let _ = self.queue.send(Job::Stop);
        if let Some(handle) = self.worker.lock().take() {
            let _ = handle.join();
        }
    }
}
