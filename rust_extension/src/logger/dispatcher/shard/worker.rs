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

use super::{IDLE_TICK, Job};
use crate::logger::QueuedRecord;
use crate::sync::{Receiver, recv_timeout};

/// Drain a shard's queue, handling each job.
pub(super) fn run_worker(jobs: Receiver<Job>) {
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
