//! One shard's queue, its in-flight permits, and the worker that drains it.
//!
//! The queue carries record, drain-barrier, and handler-flush jobs:
//! [`Job::Record`] hands a queued record to the handler set the logger captured
//! with it, [`Job::Barrier`] acknowledges a drain, and [`Job::Flush`] flushes a
//! captured handler snapshot after earlier records. Sharing the queue is what
//! keeps a flush behind the work it covers.
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
mod drain;
#[cfg(all(test, not(loom)))]
mod test_support;
mod worker;

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::handler::FemtoHandlerTrait;
use crate::logger::QueuedRecord;
use crate::logger::flush_clock::FlushDeadline;
use crate::logger::flush_diagnostics::FlushFailureStage;
use crate::sync::Receiver;
#[cfg(loom)]
use crate::sync::{JoinHandle, Mutex};
use crate::sync::{Sender, bounded};

pub(crate) use budget::LoggerBudget;
use budget::{Permit, Permits};
use worker::run_worker;

/// How long a drain-on-drop barrier waits for the shard to acknowledge it.
///
/// Flush calls supply their own deadline through [`Self::flush_handlers_until`].
/// The bound is ignored under Loom, so no model depends on it firing.
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
    /// Flush a handler snapshot after all jobs queued before this marker.
    Flush {
        /// Receives the handler snapshot after the marker is admitted.
        handlers: Receiver<Vec<Arc<dyn FemtoHandlerTrait>>>,
        /// Deadline shared by queue insertion, acknowledgement, and callbacks.
        deadline: FlushDeadline,
        /// Gate opened after the logger registry lock has been released.
        start: Receiver<()>,
        /// Reports handler success or its first unconfirmed deadline stage.
        ack: Sender<Result<(), FlushFailureStage>>,
        /// Time admission began, so the worker's reported queue wait covers
        /// waiting for room in the bounded queue as well as time behind it.
        enqueued_at: Instant,
    },
    /// Finish the jobs already queued and stop the worker.
    ///
    /// Only a caller that owns the shard sends this: a process-wide pool shard
    /// has no owner and its worker lives until the process ends.
    #[cfg(loom)]
    Stop,
}

/// How a barrier wait ended.
///
/// A caller reports the distinction rather than a bare failure: a closed
/// channel means the shard is gone for good, while a timed-out barrier means
/// the worker is still alive but did not finish the work ahead in time. The two
/// need different diagnoses, so the flush path records them separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrainOutcome {
    /// The worker handled every queued job ahead of the barrier.
    Drained,
    /// The shard's channel is closed, so no barrier can ever be answered.
    QueueClosed,
    /// The worker did not acknowledge the barrier within [`BARRIER_TIMEOUT`].
    TimedOut,
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
    /// Flush callers currently waiting for room in the bounded queue.
    ///
    /// A test that needs to know a flush caller is blocked in admission — not
    /// merely scheduled, or still on its way there — polls this. Without it the
    /// only way to tell is to sleep and hope, which passes whether or not the
    /// caller ever reached the queue.
    #[cfg(all(test, not(loom)))]
    marker_waiters: std::sync::atomic::AtomicUsize,
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
            #[cfg(all(test, not(loom)))]
            marker_waiters: std::sync::atomic::AtomicUsize::new(0),
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

    /// Build a shard with a queue but no worker, handing back the receiver.
    ///
    /// A production shard's worker lives as long as the process, so a barrier
    /// that no one answers can otherwise be observed only after a `fork`
    /// rebind failure. Dropping the returned receiver closes the queue; holding
    /// it open and filling the queue instead exercises the enqueue timeout.
    #[cfg(all(test, not(loom)))]
    fn without_worker_for_test(depth: usize) -> (Self, Receiver<Job>) {
        let (queue, jobs) = bounded::<Job>(depth);
        let shard = Self {
            queue,
            permits: Arc::new(Permits::new(depth)),
            depth,
            #[cfg(all(test, not(loom)))]
            marker_waiters: std::sync::atomic::AtomicUsize::new(0),
        };
        (shard, jobs)
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
                Job::Flush { .. } => unreachable!("only a record job is admitted here"),
                // `admit` sends nothing but records, so neither of these can
                // come back. The arm exists only because a `--cfg loom` build
                // adds `Job::Stop`, and a match over the full enum must name
                // it.
                #[cfg(loom)]
                Job::Stop => unreachable!("only a record job is admitted here"),
            },
        }
    }

    /// Return the queued job count so tests can prove a flush marker occupies
    /// the bounded queue before exercising a second flush.
    #[cfg(all(test, not(loom)))]
    pub(crate) fn queued_jobs_for_test(&self) -> usize {
        self.queue.len()
    }

    /// Count flush callers waiting for room in the bounded queue right now.
    ///
    /// A test uses this to prove a caller really is blocked in admission: a
    /// thread that has merely been spawned, or that has not reached the queue
    /// yet, is not counted, so a wait on this figure cannot pass by accident.
    #[cfg(all(test, not(loom)))]
    pub(crate) fn marker_waiters_for_test(&self) -> usize {
        self.marker_waiters
            .load(std::sync::atomic::Ordering::SeqCst)
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

// Drain-outcome tests construct a shard without a worker, which only an
// ordinary build can do: a Loom shard always owns the worker it must join.
#[cfg(all(test, not(loom)))]
#[path = "shard_tests.rs"]
mod tests;
