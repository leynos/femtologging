//! One shard's queue, its in-flight permits, and the worker that drains it.
//!
//! The queue carries two kinds of job and knows nothing about handlers:
//! [`Job::Record`] hands a queued record to the handler set the logger captured
//! with it, and [`Job::Barrier`] asks for an acknowledgement once everything
//! queued ahead of it has been handled. Sharing the queue is what lets a
//! logger's flush travel the same path as its records, so an acknowledgement
//! cannot overtake one.
//!
//! Work is bounded twice. The queue is bounded, so a burst cannot grow memory
//! without limit, and the permits bound how much work a shard holds *in
//! flight*, so a slow handler cannot turn a busy logger's backlog into an
//! unbounded one. Both refusals are returned to the caller, which counts the
//! drop and warns about it: the logging call itself never waits.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use log::warn;

use crate::logger::QueuedRecord;
#[cfg(loom)]
use crate::sync::{JoinHandle, Mutex};
use crate::sync::{Receiver, Sender, bounded, recv_timeout};

/// How long the shard worker waits for a job before looking around again.
///
/// A liveness tick, not a job deadline: queued work is returned by
/// `recv_timeout` as soon as it arrives, so this only bounds how long an idle
/// shard waits before noticing that its queue has been dropped. Under Loom the
/// bound is ignored, and Loom's scheduler decides when the worker looks.
const IDLE_TICK: Duration = Duration::from_millis(2_000);

/// Expose the idle tick so a test can deliberately outlast it.
///
/// Outside Loom only, for the same reason as the re-export above it.
#[cfg(all(test, not(loom)))]
pub(crate) fn idle_tick_for_test() -> Duration {
    IDLE_TICK
}

/// How long a barrier waits for the shard to acknowledge it.
///
/// Bounded because a barrier is waited on from `Drop` and from
/// `flush_handlers`: a handler that never returns must delay those calls, not
/// hang them. A timed-out barrier loses the queued records, which is what the
/// per-logger worker did when its own flush timed out. As with the tick, the
/// bound is ignored under Loom, so no model depends on it firing.
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
        queued: Box<QueuedRecord>,
        permit: Permit,
    },
    /// Signal `ack` once every job ahead of this one has been handled.
    Barrier { ack: Sender<()> },
    /// Finish the jobs already queued and stop the worker.
    ///
    /// Only a caller that owns the shard sends this: a process-wide pool shard
    /// has no owner and its worker lives until the process ends.
    #[cfg(loom)]
    Stop,
}

/// One shard's backlog, and the permits bounding what it holds.
pub(crate) struct Shard {
    queue: Sender<Job>,
    /// One free token per slot of in-flight work.
    permits: Arc<Permits>,
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

/// The permits bounding how much work one shard holds.
struct Permits {
    /// One token per free in-flight slot, taken non-blockingly by a producer.
    available: Receiver<()>,
    /// Where a used token goes back.
    ///
    /// Kept alongside `available` so the channel's senders are never all
    /// dropped: a disconnected channel is not an exhausted pool, and the two
    /// would otherwise be indistinguishable to `try_recv`.
    returned: Sender<()>,
}

/// Permission to hold one record in flight on a shard.
///
/// It travels to the worker inside the job it authorises, and returns to the
/// pool when that job is dropped — after the handlers have run, or while a
/// panicking handler unwinds. A permit therefore counts its record as in flight
/// for exactly as long as the record is unhandled, on every path out.
struct Permit {
    permits: Arc<Permits>,
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
    fn new(capacity: usize) -> Self {
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

impl Permit {
    /// Take a permit if the shard has one free.
    ///
    /// Never waits: a shard that has handed out every permit is already holding
    /// as much work as it will, so the caller counts a dropped record rather
    /// than blocking the logging call.
    fn take(permits: &Arc<Permits>) -> Option<Self> {
        permits.available.try_recv().ok().map(|()| Self {
            permits: Arc::clone(permits),
        })
    }
}

impl Drop for Permit {
    /// Return the permit to its pool.
    ///
    /// Releasing from `Drop` rather than a named method is what makes the
    /// accounting hold on every path out of a job, including one where a
    /// handler panics and unwinds through it.
    fn drop(&mut self) {
        // A failed send can only mean the pool is already full, which would be
        // a double release; ignoring it keeps the count from inflating.
        let _ = self.permits.returned.try_send(());
    }
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
            #[cfg(loom)]
            worker: Mutex::new(Some(_worker)),
        })
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
    /// full or gone, so the caller can count the drop and warn about it without
    /// paying for an allocation on the path that runs when the process is
    /// already short of a resource.
    pub(crate) fn admit(&self, queued: Box<QueuedRecord>) -> Result<(), Box<QueuedRecord>> {
        let Some(permit) = Permit::take(&self.permits) else {
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
    pub(crate) fn wait_for_drain(&self) -> bool {
        let (ack, acked) = bounded::<()>(1);
        if self
            .queue
            .send_timeout(Job::Barrier { ack }, BARRIER_TIMEOUT)
            .is_err()
        {
            return false;
        }
        recv_timeout(&acked, BARRIER_TIMEOUT).is_ok()
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

/// Drain a shard's queue, handling each job.
///
/// A panicking handler must not stop the shard, so [`handle_job`] catches a
/// panic per handler and the loop continues: one logger's misbehaving handler
/// neither silences the loggers sharing the shard nor deprives its own record's
/// other handlers. The catch here is a backstop for a panic outside a handler
/// call — a failure in the shard's own bookkeeping — so the worker survives
/// that too rather than ending the shard for every logger on it.
fn run_worker(jobs: Receiver<Job>) {
    loop {
        let job = match recv_timeout(&jobs, IDLE_TICK) {
            Ok(job) => job,
            // A timeout is the liveness tick, not the end of the shard.
            // Ending the loop here would retire every worker the first time
            // the process logged nothing for `IDLE_TICK`, and every later
            // flush would wait on a barrier nobody was left to answer.
            Err(crate::sync::RecvTimeoutError::Timeout) => continue,
            // Only a dropped queue — every sender gone — ends a worker.
            Err(crate::sync::RecvTimeoutError::Disconnected) => break,
        };
        #[cfg(loom)]
        if matches!(job, Job::Stop) {
            break;
        }
        if catch_unwind(AssertUnwindSafe(|| handle_job(job))).is_err() {
            warn!("FemtoLogger: dispatch worker recovered from a panic while handling a job");
        }
    }
}

/// Perform one job.
fn handle_job(job: Job) {
    match job {
        Job::Record { queued, permit } => {
            let QueuedRecord {
                record,
                handlers,
                #[cfg(feature = "python")]
                context,
            } = *queued;
            // Caught per handler, not around the whole loop: a handler that
            // panics is one handler's bug, and the record is still the other
            // handlers' to receive. Catching only around the job would make the
            // panicking handler's neighbours lose a record they had every
            // reason to expect.
            for handler in &handlers {
                #[cfg(feature = "tracing-compat")]
                let dispatch_started = std::time::Instant::now();
                #[cfg(feature = "tracing-compat")]
                let handler_kind = if handler.is_python_backed() {
                    "python"
                } else {
                    "native"
                };
                #[cfg(feature = "python")]
                let result = catch_unwind(AssertUnwindSafe(|| {
                    handler.handle_with_context(record.clone(), context.as_ref())
                }));
                #[cfg(not(feature = "python"))]
                let result = catch_unwind(AssertUnwindSafe(|| handler.handle(record.clone())));
                #[cfg(feature = "tracing-compat")]
                let dispatch_outcome = match &result {
                    Ok(Ok(())) => "success",
                    Ok(Err(_)) => "failed",
                    Err(_) => "panicked",
                };
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => warn!("FemtoLogger: handler reported an error: {error}"),
                    Err(_) => {
                        warn!("FemtoLogger: dispatch worker recovered from a panicking handler")
                    }
                }
                #[cfg(feature = "tracing-compat")]
                tracing::trace!(
                    target: "femtologging::internal",
                    operation = "worker_handler_dispatch",
                    handler_kind,
                    dispatch_outcome,
                    elapsed_us = dispatch_started.elapsed().as_micros(),
                );
            }
            // Released only now: the permit counts this record as in flight
            // until its handlers are done with it. A panic that escapes the
            // per-handler catch unwinds through here and drops the permit
            // anyway, so the accounting survives that path too.
            drop(permit);
        }
        Job::Barrier { ack } => {
            let _ = ack.try_send(());
        }
        #[cfg(loom)]
        Job::Stop => unreachable!("the worker stops before it handles a stop job"),
    }
}
