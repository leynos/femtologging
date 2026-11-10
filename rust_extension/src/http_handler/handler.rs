//! Public handler type exported by the crate.

use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

#[cfg(feature = "python")]
use pyo3::prelude::*;
// The interpreter is reachable in every lane that is not modelling it, so the
// drop guard attaches under the same condition `FemtoLogger`'s join does.
#[cfg(not(loom))]
use pyo3::Python;

use parking_lot::Mutex;

use crate::{
    handler::{FemtoHandlerTrait, FlushContext, HandlerError, capped_deadline, deadline_after},
    log_record::FemtoLogRecord,
    rate_limited_warner::RateLimitedWarner,
    shutdown_signal::ShutdownSignal,
};

#[cfg(test)]
use crate::test_utils::test_worker::TestWorker;

use super::{
    config::HTTPHandlerConfig,
    queue::{HTTPCommand, enqueue_record},
    worker::spawn_worker,
};

mod lifecycle;

#[cfg(test)]
#[path = "handler/flush_deadline_tests.rs"]
mod flush_deadline_tests;

use lifecycle::ShutdownOutcome;

#[cfg_attr(feature = "python", pyclass)]
/// Handler forwarding records to an HTTP endpoint.
///
/// Supports URL-encoded form data (CPython parity) and JSON serialization.
/// Uses exponential backoff for transient failures (5xx, 429, network errors)
/// and drops records on permanent failures (4xx except 429).
pub struct FemtoHTTPHandler {
    /// Sends records and lifecycle commands to the worker.
    tx: Option<crossbeam_channel::Sender<HTTPCommand>>,
    /// Owns the worker until it finishes or is detached after a timeout.
    handle: Mutex<Option<thread::JoinHandle<()>>>,
    /// Counts dropped records and limits warning frequency.
    warner: RateLimitedWarner,
    /// The shutdown signal shared with the worker.
    ///
    /// `Some` for as long as the handler is open. `None` once `close` has
    /// given up on the worker, or after a close that had nothing to do.
    signal: Option<Arc<ShutdownSignal>>,
    /// Timeout for flush and shutdown operations.
    ///
    /// Derived from `write_timeout` in the configuration: a flush or graceful
    /// shutdown should complete within the same time bounds as a single HTTP
    /// request.
    flush_timeout: Duration,
}

impl FemtoHTTPHandler {
    /// Construct the handler from a configuration object.
    pub fn with_config(config: HTTPHandlerConfig) -> Self {
        let flush_timeout = config.write_timeout;
        let warner = RateLimitedWarner::new(config.warn_interval);
        let signal = Arc::new(ShutdownSignal::new());
        let (tx, handle) = spawn_worker(config, Arc::clone(&signal));
        Self {
            tx: Some(tx),
            handle: Mutex::new(Some(handle)),
            warner,
            signal: Some(signal),
            flush_timeout,
        }
    }

    /// Flush any pending log records.
    pub fn flush(&self) -> bool {
        let now = Instant::now();
        let Some(deadline) = deadline_after(now, self.flush_timeout) else {
            return false;
        };
        self.flush_by(&FlushContext::system(now, Some(deadline)))
    }

    /// Flush any pending log records, waiting no longer than the context's
    /// deadline.
    ///
    /// The wait also stops at this handler's configured `flush_timeout`, so a
    /// bare `flush()` keeps the bound it documents and a barrier flush cannot
    /// outlive the deadline it was given. Time is sampled from the context, so
    /// a barrier test controls this handler's deadline transition as well as
    /// the logger's.
    fn flush_by(&self, context: &FlushContext) -> bool {
        let Some(deadline) = context.deadline() else {
            return false;
        };
        self.flush_by_at(deadline, &mut || context.now())
    }

    /// Flush against `deadline`, sampling each queue wait from `now`.
    ///
    /// Production passes [`Instant::now`] here; tests pass a scripted source,
    /// so a stalled worker queue can be shown to consume exactly the budget
    /// the caller supplied rather than a fresh copy for each wait.
    fn flush_by_at(&self, deadline: Instant, now: &mut dyn FnMut() -> Instant) -> bool {
        let Some(tx) = self.sender() else {
            return false;
        };
        let handler_deadline = capped_deadline(deadline, now(), self.flush_timeout);
        let outcome =
            self.warner
                .flush_with_pending_drops(handler_deadline, now, |pending_drops, budget| {
                    let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
                    tx.send_timeout(HTTPCommand::Flush(ack_tx, pending_drops), budget)
                        .ok()?;
                    Some(move |budget| ack_rx.recv_timeout(budget).ok())
                });
        matches!(outcome, Ok(()))
    }

    /// Close the handler, waiting for the worker only while it is answering.
    ///
    /// One deadline covers the whole operation: the shutdown signal, the
    /// command that carries the acknowledgement request, the acknowledgement
    /// itself, and the join. A worker that does not acknowledge within that
    /// budget is abandoned rather than joined. Joining it would wait for
    /// however long it had left to run, which for this handler is the backoff
    /// deadline, a figure `flush_timeout` does not describe and the caller did
    /// not ask for. `close` runs from `Drop`, so that wait would fall on
    /// whatever was tearing the handler down.
    ///
    /// The signal is what makes the budget apply to the work and not only to
    /// the waiting. Until it is raised the worker is free to retry a
    /// poorly-answered record for as long as its backoff allows; once it is,
    /// no new attempt begins after the deadline, each queued record gets one
    /// attempt, and whatever is still queued at the deadline is discarded with
    /// a warning rather than sent.
    ///
    /// This is the contract [`FemtoStreamHandler::close`] already documents
    /// and implements: acknowledge or be abandoned, with the timeout warned
    /// about rather than swallowed. Two handlers with the same lifecycle
    /// should not answer the same question differently.
    ///
    /// [`FemtoStreamHandler::close`]: crate::FemtoStreamHandler::close
    pub fn close(&mut self) {
        match self.request_shutdown() {
            ShutdownOutcome::Finished => self.join_worker(),
            ShutdownOutcome::TimedOut => self.abandon_worker(),
        }
    }

    /// Build a handler around a worker the caller supplies.
    ///
    /// Exists so the close path can be tested against a worker that leaves on
    /// the signal without ever reading the shutdown command - a real worker
    /// does that when it is asleep when the signal is raised. Racing a live
    /// worker for that window is not possible to do deterministically, so the
    /// window is constructed instead.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let (tx, rx) = crossbeam_channel::bounded(1);
    /// let (signal, handle) = spawn_test_worker(rx);
    /// let mut handler = FemtoHTTPHandler::with_worker_for_test(
    ///     tx,
    ///     TestWorker::new(signal, handle),
    ///     Duration::from_millis(200),
    ///     RateLimitedWarner::new(Duration::from_secs(1)),
    /// );
    /// ```
    #[cfg(test)]
    pub(crate) fn with_worker_for_test(
        tx: crossbeam_channel::Sender<HTTPCommand>,
        worker: TestWorker,
        flush_timeout: Duration,
        warner: RateLimitedWarner,
    ) -> Self {
        Self {
            tx: Some(tx),
            handle: Mutex::new(Some(worker.handle)),
            warner,
            signal: Some(worker.signal),
            flush_timeout,
        }
    }

    /// Clone the live worker sender, if the handler has not closed.
    ///
    /// Example: a missing sender makes `handle` return `HandlerError::Closed`.
    fn sender(&self) -> Option<crossbeam_channel::Sender<HTTPCommand>> {
        self.tx.as_ref().cloned()
    }
}

#[cfg(feature = "python")]
#[pymethods]
impl FemtoHTTPHandler {
    /// Convert Python arguments to a record and enqueue it for delivery.
    ///
    /// Example: `handler.handle("app", "INFO", "started")` queues one record.
    #[pyo3(name = "handle")]
    fn py_handle(&self, logger: &str, level: &str, message: &str) -> PyResult<()> {
        let parsed_level = crate::level::FemtoLevel::parse_py(level)?;
        self.handle(FemtoLogRecord::new(logger, parsed_level, message))
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Handler error: {e}")))
    }

    /// Flush pending log records without shutting down the worker thread.
    ///
    /// The flush timeout equals the ``write_timeout`` configured on the
    /// handler (default: 30 seconds).
    ///
    /// Returns
    /// -------
    /// bool
    ///     ``True`` when the worker acknowledges the flush within the
    ///     configured timeout.
    ///     ``False`` when the handler has already been closed, the
    ///     internal channel to the worker has been dropped, or the worker
    ///     does not acknowledge before the timeout elapses.
    ///
    /// Examples
    /// --------
    /// >>> handler.flush()
    /// True
    /// >>> handler.close()
    /// >>> handler.flush()
    /// False
    #[pyo3(name = "flush")]
    fn py_flush(&self, py: Python<'_>) -> bool {
        py.detach(|| self.flush())
    }

    /// Close the handler, waiting for the worker at most ``write_timeout``.
    ///
    /// A worker that acknowledges the shutdown within ``write_timeout`` is
    /// joined. One that does not, typically because a request already in
    /// flight is waiting on an unresponsive endpoint, is abandoned with a warning
    /// rather than waited for. Once abandoned it stops rather than draining:
    /// the records still queued are discarded with a warning, because sending
    /// them is what would have taken the time ``write_timeout`` just ran out
    /// of.
    ///
    /// The wait runs with the GIL released, so a close that spends its budget
    /// waiting does not hold the interpreter while it does.
    #[pyo3(name = "close")]
    fn py_close(&mut self, py: Python<'_>) {
        py.detach(|| self.close());
    }
}

impl FemtoHandlerTrait for FemtoHTTPHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let Some(tx) = self.sender() else {
            self.warner.record_drop();
            self.warner.warn_if_due(|count| {
                log::warn!("FemtoHTTPHandler dropped {count} records after shutdown");
            });
            return Err(HandlerError::Closed);
        };
        enqueue_record(&tx, record, &self.warner)
    }

    fn flush(&self) -> bool {
        FemtoHTTPHandler::flush(self)
    }

    fn flush_until(&self, context: &FlushContext) -> bool {
        self.flush_by(context)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Drop the handler, closing it with the GIL released.
///
/// A handler whose worker has already gone has nothing to close, so the check
/// keeps that case out of the GIL's way entirely. When Python can be attached,
/// the close runs detached; during interpreter shutdown, the Rust-only close
/// runs without attaching and remains bounded by the same budget.
///
/// Under `--cfg loom` there is no interpreter to attach to: no model builds one
/// of these handlers, and attaching would initialize Python on a Loom
/// coroutine's small stack.
#[cfg(not(loom))]
impl Drop for FemtoHTTPHandler {
    fn drop(&mut self) {
        if self.handle.lock().is_none() {
            return;
        }
        if Python::try_attach(|py| py.detach(|| self.close())).is_none() {
            self.close();
        }
    }
}

#[cfg(loom)]
impl Drop for FemtoHTTPHandler {
    fn drop(&mut self) {
        self.close();
    }
}

impl std::fmt::Debug for FemtoHTTPHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FemtoHTTPHandler")
            .field("flush_timeout", &self.flush_timeout)
            .finish()
    }
}
