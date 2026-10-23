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

use crate::handler::{FemtoHandlerTrait, remaining};
use super::Job;
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
        Job::Flush {
            handlers,
            deadline,
            start,
            ack,
        } => {
            let result = start
                .recv_timeout(remaining(deadline))
                .is_ok()
                && flush_handlers(&handlers, deadline);
            let _ = ack.try_send(result);
        }
        #[cfg(loom)]
        Job::Stop => unreachable!("the worker stops before it handles a stop job"),
    }
}

/// Flush each captured handler while the caller's deadline remains live.
fn flush_handlers(handlers: &[std::sync::Arc<dyn FemtoHandlerTrait>], deadline: std::time::Instant) -> bool {
    for handler in handlers {
        if remaining(deadline).is_zero() || !handler.flush_until(deadline) {
            return false;
        }
        if std::time::Instant::now() > deadline {
            return false;
        }
    }
    std::time::Instant::now() <= deadline
}
