//! The loop that drains one shard's queue.
//!
//! A panicking handler must not stop the shard, so a panic is caught per
//! handler and the loop continues: one logger's misbehaving handler neither
//! silences the loggers sharing the shard nor deprives its own record's other
//! handlers. The catch around the whole job is a backstop for a panic outside a
//! handler call — a failure in the shard's own bookkeeping — so the worker
//! survives that too rather than ending the shard for every logger on it.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use log::warn;

use super::Job;
use crate::handler::FemtoHandlerTrait;
use crate::logger::QueuedRecord;
use crate::sync::Receiver;

/// Drain a shard's queue, handling each job.
///
/// The receive blocks until a job arrives or the queue is gone. A bounded
/// wait would add a timeout arm that means nothing — a quiet queue is not a
/// stopped shard — and treating that arm as shutdown retires every worker the
/// first time the process logs nothing for the bound, leaving the next flush
/// to wait on a barrier nobody is left to answer. Blocking leaves only one way
/// out, so that mistake cannot be written here.
pub(super) fn run_worker(jobs: &Receiver<Job>) {
    // `recv` fails only once the queue is empty *and* every sender is gone:
    // crossbeam wakes a blocked receiver when the last sender drops, so the
    // end of the shard is still noticed without a tick to poll for it.
    while let Ok(job) = jobs.recv() {
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
            // Caught per handler, not around the whole loop: a handler that
            // panics is one handler's bug, and the record is still the other
            // handlers' to receive. Catching only around the job would make the
            // panicking handler's neighbours lose a record they had every
            // reason to expect.
            for handler in &queued.handlers {
                dispatch_to_handler(handler, &queued);
            }
            // Released only now: the permit counts this record as in flight
            // until its handlers are done with it. A panic that escapes the
            // per-handler catch unwinds through here and drops the permit
            // anyway, so the accounting survives that path too.
            drop(permit);
        }
        Job::Barrier { ack } => {
            // A failed acknowledgement means the waiter has already given up:
            // a full channel cannot happen for a one-shot ack, and a closed
            // one is the flush that queued it having timed out. There is
            // nobody left to tell, so the outcome is recorded and dropped.
            if ack.try_send(()).is_err() {
                log::trace!("FemtoLogger: barrier waiter left before the acknowledgement");
            }
        }
        #[cfg(loom)]
        Job::Stop => unreachable!("the worker stops before it handles a stop job"),
    }
}

/// Deliver one record to one handler, reporting a failure or a panic.
///
/// A handler's panic is caught here rather than escaping the job so the
/// record's remaining handlers still receive it.
fn dispatch_to_handler(handler: &Arc<dyn FemtoHandlerTrait>, queued: &QueuedRecord) {
    #[cfg(feature = "tracing-compat")]
    let dispatch_started = std::time::Instant::now();
    #[cfg(feature = "tracing-compat")]
    let handler_kind = if handler.is_python_backed() {
        "python"
    } else {
        "native"
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        #[cfg(feature = "python")]
        {
            handler.handle_with_context(queued.record.clone(), queued.context.as_ref())
        }
        #[cfg(not(feature = "python"))]
        {
            handler.handle(queued.record.clone())
        }
    }));
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
            warn!("FemtoLogger: dispatch worker recovered from a panicking handler");
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
