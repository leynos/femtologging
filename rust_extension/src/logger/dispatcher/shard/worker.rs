//! The loop that drains one shard's queue.
//!
//! A panicking handler must not stop the shard, so a panic is caught per
//! handler and the loop continues: one logger's misbehaving handler neither
//! silences the loggers sharing the shard nor deprives its own record's other
//! handlers. The catch around the whole job is a backstop for a panic outside a
//! handler call — a failure in the shard's own bookkeeping — so the worker
//! survives that too rather than ending the shard for every logger on it.

use std::panic::{AssertUnwindSafe, catch_unwind};

use log::warn;

use super::Job;
use super::budget::Permit;
use crate::handler::remaining;
use crate::logger::QueuedRecord;
use crate::logger::flush::flush_handler_snapshot_at;
use crate::logger::flush_diagnostics::FlushFailureStage;
use crate::sync::{Receiver, RecvTimeoutError};

/// Drain a shard's queue, handling each job.
///
/// The receive blocks until a job arrives or the queue is gone. A bounded
/// wait would add a timeout arm that means nothing — a quiet queue is not a
/// stopped shard — and treating that arm as shutdown retires every worker the
/// first time the process logs nothing for the bound, leaving the next flush
/// to wait on a barrier nobody is left to answer. Blocking leaves only one way
/// out, so that mistake cannot be written here.
pub(super) fn run_worker(jobs: Receiver<Job>) {
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
        Job::Record { queued, permit } => handle_record_job(queued, permit),
        Job::Barrier { ack } => {
            let _ = ack.try_send(());
        }
        Job::Flush {
            handlers,
            deadline,
            start,
            ack,
        } => {
            let result = match start.recv_timeout(remaining(deadline, std::time::Instant::now())) {
                Ok(()) => flush_handler_snapshot_at(&handlers, deadline, std::time::Instant::now),
                Err(RecvTimeoutError::Timeout) => Err(FlushFailureStage::WorkerStartTimedOut),
                Err(RecvTimeoutError::Disconnected) => {
                    Err(FlushFailureStage::WorkerStartDisconnected)
                }
            };
            let _ = ack.try_send(result);
        }
        #[cfg(loom)]
        Job::Stop => unreachable!("the worker stops before it handles a stop job"),
    }
}

/// Dispatch one record to its captured handlers, then release its permit.
fn handle_record_job(queued: Box<QueuedRecord>, permit: Permit) {
    let QueuedRecord {
        record,
        handlers,
        #[cfg(feature = "python")]
        context,
    } = *queued;
    // Caught per handler, not around the whole loop: a handler that panics is
    // one handler's bug, and the record is still the other handlers' to
    // receive. Catching only around the job would make the panicking handler's
    // neighbours lose a record they had every reason to expect.
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
            Err(_) => warn!("FemtoLogger: dispatch worker recovered from a panicking handler"),
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
    // The permit counts this record as in flight until its handlers finish. If
    // a panic escapes a per-handler catch, unwinding drops the permit as well.
    drop(permit);
}
