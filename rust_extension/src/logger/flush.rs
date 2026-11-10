//! Synchronous handler flushing for [`super::FemtoLogger`].
//!
//! A flush has two halves: the logger's own queued records must be handled
//! before the handlers are asked to flush, and the handlers must then be
//! flushed without holding any lock a handler could need. Both halves are
//! timed and classified once, so the counters and the tracing span describe
//! the same attempt.

use std::sync::Arc;
use std::time::Instant;

use crate::handler::FemtoHandlerTrait;

use super::DrainOutcome;
use super::FemtoLogger;
use super::flush_metrics::{self, FlushOutcome};

impl FemtoLogger {
    /// Flush all handlers attached to this logger.
    ///
    /// This call blocks until the worker acknowledges a flush marker and each
    /// attached handler has flushed. Enqueueing the flush marker and waiting
    /// for its acknowledgement share a two-second timeout. Flushing attached
    /// handlers can take additional time.
    ///
    /// When Python handlers can be attached, callers that currently hold
    /// Python access must detach before calling this method. Otherwise the
    /// worker cannot acquire the GIL to run a Python handler.
    ///
    /// # Returns
    ///
    /// `true` if the worker acknowledges the flush marker within the shared
    /// timeout and every attached handler flush succeeds. `false` if the
    /// marker cannot be enqueued or acknowledged in time, or a handler flush
    /// fails.
    ///
    /// # Examples
    ///
    /// A Rust caller holding Python access detaches during the blocking flush:
    ///
    /// ```
    /// use _femtologging_rs::FemtoLogger;
    /// use pyo3::Python;
    ///
    /// let logger = FemtoLogger::new(String::from("example"))
    ///     .expect("the shared dispatch pool starts");
    /// let flushed = Python::attach(|py| py.detach(|| logger.flush_handlers()));
    /// assert!(flushed);
    /// ```
    pub fn flush_handlers(&self) -> bool {
        #[cfg(all(feature = "python", feature = "test-util"))]
        self.flush_test_hooks.signal_entered();

        self.flush_handlers_blocking()
    }

    /// Drain this logger's queued records, then flush its configured handlers.
    ///
    /// The whole attempt is timed and classified once, so the counters and the
    /// tracing span describe the same flush. The span is opened before the
    /// drain so an external subscriber sees the wait as part of the operation;
    /// its outcome and duration fields are recorded when the attempt ends.
    pub(super) fn flush_handlers_blocking(&self) -> bool {
        let started = Instant::now();
        #[cfg(feature = "tracing-compat")]
        let span = tracing::trace_span!(
            target: "femtologging::internal",
            "logger_flush",
            operation = "logger_flush",
            flush_outcome = tracing::field::Empty,
            elapsed_us = tracing::field::Empty,
        );
        #[cfg(feature = "tracing-compat")]
        let _entered = span.enter();

        let outcome = self.flush_handlers_inner();
        let elapsed = started.elapsed();
        #[cfg(feature = "tracing-compat")]
        {
            span.record("flush_outcome", outcome.label());
            span.record("elapsed_us", flush_metrics::elapsed_micros(elapsed));
        }
        flush_metrics::record_flush(outcome, elapsed);
        outcome.is_success()
    }

    /// Classify one flush attempt by its terminal outcome.
    fn flush_handlers_inner(&self) -> FlushOutcome {
        if let Some(failure) = FlushOutcome::from_drain(self.wait_for_dispatch_to_drain()) {
            return failure;
        }
        if self.flush_configured_handlers() {
            FlushOutcome::Success
        } else {
            FlushOutcome::HandlerFailure
        }
    }

    /// Wait for the logger's shard to finish the records queued ahead.
    ///
    /// The barrier travels the shard's own queue, so it cannot overtake a
    /// record this logger logged first, and a shard that is gone or too slow
    /// reports [`DrainOutcome::QueueClosed`] or [`DrainOutcome::TimedOut`]
    /// rather than waiting indefinitely.
    ///
    /// A logger with nothing in flight has nothing of its own to drain, so it
    /// answers immediately. The budget releases a record's charge only after
    /// its handlers have run, so a count of zero means every record this
    /// logger queued has been handled. Queuing a barrier anyway would cost more
    /// than the round trip: the `log` bridge's `flush_all_handlers` sweeps
    /// every registered logger, and a barrier takes a queue slot but no
    /// permit, so barriers can crowd out the records they were meant to drain
    /// and make `admit` refuse work the shard had room for. `Drop` already
    /// skips its wait this way.
    ///
    /// The wait is taken with the interpreter detached. `flush_handlers` is a
    /// Python method, so its caller holds the GIL, and a queued Python handler
    /// needs that same GIL to finish — holding it here would make the wait time
    /// out against a handler that could not run. Detaching releases the GIL for
    /// the duration, and the wait is taken directly when there is no
    /// interpreter to detach from, which is also when no Python handler can be
    /// mid-call.
    fn wait_for_dispatch_to_drain(&self) -> DrainOutcome {
        #[cfg(not(loom))]
        let (shard, budget) = match self.dispatch_state() {
            Ok(dispatch) => dispatch,
            // A logger whose pool cannot be reached has no shard to drain, so
            // its barrier can never be answered.
            Err(_) => return DrainOutcome::QueueClosed,
        };
        #[cfg(not(loom))]
        if budget.in_flight() == 0 {
            return DrainOutcome::Drained;
        }
        #[cfg(not(loom))]
        if let Some(drained) =
            pyo3::Python::try_attach(|py| py.detach(|| shard.wait_for_drain_outcome()))
        {
            return drained;
        }
        #[cfg(not(loom))]
        return shard.wait_for_drain_outcome();
        #[cfg(loom)]
        self.shard.wait_for_drain_outcome()
    }

    /// Flush every configured handler and report whether all succeeded.
    ///
    /// The handler list is snapshotted under the read guard and the guard is
    /// released before any handler runs. A handler's flush may be arbitrary
    /// Python, and Python code is free to log through this logger or to add and
    /// remove handlers; holding the read guard across that call would stall
    /// those writers, and a writer that then waits on the flush would deadlock.
    ///
    /// Every handler is flushed even after one reports failure. A flush is
    /// best-effort: one broken handler must not leave the records buffered in
    /// the handlers after it, so the result is the conjunction of all attempts
    /// rather than the first failure.
    fn flush_configured_handlers(&self) -> bool {
        let handlers: Vec<Arc<dyn FemtoHandlerTrait>> = {
            let attachments = self.handlers.read();
            attachments
                .iter()
                .map(|attachment| Arc::clone(&attachment.handler))
                .collect()
        };
        let mut all_succeeded = true;
        for handler in handlers {
            if !handler.flush() {
                all_succeeded = false;
            }
        }
        all_succeeded
    }
}
