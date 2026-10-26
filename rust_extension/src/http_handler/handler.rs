//! Public handler type exported by the crate.

use std::{
    thread,
    time::{Duration, Instant},
};

#[cfg(feature = "python")]
use pyo3::prelude::*;

use parking_lot::Mutex;

use crate::{
    handler::{FemtoHandlerTrait, HandlerError, capped_deadline, remaining},
    log_record::FemtoLogRecord,
    rate_limited_warner::RateLimitedWarner,
    sync::{Sender, bounded},
};

use super::{
    config::HTTPHandlerConfig,
    worker::{HTTPCommand, enqueue_record, spawn_worker},
};

/// Why a flush command could not be confirmed by the HTTP worker.
enum FlushFailure {
    /// The bounded command queue did not accept the flush before its deadline.
    Enqueue,
    /// The worker accepted the command but did not acknowledge it in time.
    Acknowledgement,
}

/// Sends a flush command to the HTTP worker and waits for acknowledgement.
///
/// Both queue waits use the caller's deadline. The worker emits `pending_drops`
/// after it sends the acknowledgement, keeping synchronous warning callbacks
/// off the waiting thread.
///
/// # Errors
///
/// Returns [`FlushFailure::Enqueue`] if the bounded queue rejects the command,
/// or [`FlushFailure::Acknowledgement`] if the worker does not acknowledge it
/// by `deadline`.
fn flush_queue(
    tx: &Sender<HTTPCommand>,
    deadline: Instant,
    pending_drops: Option<u64>,
) -> Result<(), FlushFailure> {
    let (ack_tx, ack_rx) = bounded(1);
    if tx
        .send_timeout(
            HTTPCommand::Flush(ack_tx, pending_drops),
            remaining(deadline),
        )
        .is_err()
    {
        return Err(FlushFailure::Enqueue);
    }
    ack_rx
        .recv_timeout(remaining(deadline))
        .map(|_| ())
        .map_err(|_| FlushFailure::Acknowledgement)
}

#[cfg_attr(feature = "python", pyclass)]
/// Handler forwarding records to an HTTP endpoint.
///
/// Supports URL-encoded form data (CPython parity) and JSON serialization.
/// Uses exponential backoff for transient failures (5xx, 429, network errors)
/// and drops records on permanent failures (4xx except 429).
pub struct FemtoHTTPHandler {
    /// Sender for records and flush/shutdown commands; taking it closes new delivery.
    tx: Option<crossbeam_channel::Sender<HTTPCommand>>,
    /// Join handle retained behind a mutex so close and drop can consume it once.
    handle: Mutex<Option<thread::JoinHandle<()>>>,
    /// Counts records rejected after shutdown or queue failure for rate-limited warnings.
    warner: RateLimitedWarner,
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
        let (tx, handle) = spawn_worker(config);
        Self {
            tx: Some(tx),
            handle: Mutex::new(Some(handle)),
            warner,
            flush_timeout,
        }
    }

    /// Flush any pending log records.
    pub fn flush(&self) -> bool {
        self.flush_by(Instant::now() + self.flush_timeout)
    }

    /// Flush any pending log records, waiting no longer than `deadline`.
    ///
    /// The wait also stops at this handler's configured `flush_timeout`, so a
    /// bare [`FemtoHTTPHandler::flush`] keeps the bound it documents and a
    /// barrier flush cannot outlive the deadline it was given.
    fn flush_by(&self, deadline: Instant) -> bool {
        let Some(tx) = self.sender() else {
            return false;
        };
        let pending_drops = self.warner.take_due();
        let handler_deadline = capped_deadline(deadline, self.flush_timeout);
        match flush_queue(&tx, handler_deadline, pending_drops) {
            Ok(()) => true,
            Err(FlushFailure::Enqueue) => {
                if let Some(count) = pending_drops {
                    self.warner.restore_drops(count);
                }
                false
            }
            Err(FlushFailure::Acknowledgement) => false,
        }
    }

    /// Close the handler, waiting for the worker only while it is answering.
    ///
    /// The shutdown request is bounded by `flush_timeout`, and so is the whole
    /// of `close`: a worker that does not acknowledge within that budget is
    /// abandoned rather than joined. Joining it would wait for however long it
    /// had left to run, which for this handler is the backoff deadline, a
    /// figure `flush_timeout` does not describe and the caller did not ask
    /// for. `close` runs from `Drop`, so that wait would fall on whatever was
    /// tearing the handler down.
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

    /// Clones the live queue sender, or returns `None` once shutdown has taken it.
    fn sender(&self) -> Option<crossbeam_channel::Sender<HTTPCommand>> {
        self.tx.as_ref().cloned()
    }

    /// Ask the worker to stop, and report whether joining it is now bounded.
    ///
    /// The distinction the return value carries is the whole point: every
    /// outcome except a timeout means the worker has left, or is leaving, its
    /// command loop, so the join that follows is short. An earlier version
    /// discarded this with `let _ =`, which is what let an unbounded join
    /// follow a bounded wait.
    ///
    /// One deadline covers both halves. The command channel is bounded, so
    /// when it is full the send itself blocks until the worker next reads it,
    /// which a worker stuck mid-request does not do; a send outside the budget
    /// would put that whole wait back in front of the bounded receive.
    fn request_shutdown(&mut self) -> ShutdownOutcome {
        let Some(tx) = self.tx.take() else {
            // Already closed. There is no worker left to wait for.
            return ShutdownOutcome::Finished;
        };
        let deadline = Instant::now() + self.flush_timeout;
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        match tx.send_deadline(HTTPCommand::Shutdown(ack_tx), deadline) {
            Ok(()) => {}
            // The worker has dropped its receiver, so it has already left the
            // loop; joining it returns at once and may report a panic.
            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => {
                return ShutdownOutcome::Finished;
            }
            // The queue stayed full for the whole budget: the worker is not
            // reading commands, so it could not have acknowledged either.
            Err(crossbeam_channel::SendTimeoutError::Timeout(_)) => {
                return ShutdownOutcome::TimedOut;
            }
        }
        match ack_rx.recv_deadline(deadline) {
            Ok(()) => ShutdownOutcome::Finished,
            // The worker dropped the acknowledgement channel without using it,
            // which it can only do on its way out.
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => ShutdownOutcome::Finished,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => ShutdownOutcome::TimedOut,
        }
    }

    /// Takes and joins the worker handle, warning if the worker panicked.
    fn join_worker(&mut self) {
        let Some(handle) = self.handle.lock().take() else {
            return;
        };
        if handle.join().is_err() {
            log::warn!("FemtoHTTPHandler: worker thread panicked");
        }
    }

    /// Detach the worker after it failed to acknowledge shutdown.
    ///
    /// The handle is taken so nothing joins it later, in particular the `Drop`
    /// that may follow this call. The thread itself ends on its own once its
    /// in-flight request and backoff finish, because its command channel is
    /// already disconnected.
    fn abandon_worker(&mut self) {
        log::warn!(
            "FemtoHTTPHandler: worker did not acknowledge shutdown within {:?}; \
             abandoning it rather than blocking the caller",
            self.flush_timeout
        );
        self.handle.lock().take();
    }
}

/// What asking the worker to stop established about joining it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownOutcome {
    /// The worker acknowledged, or had already left its loop. The join is
    /// bounded and worth doing, because it is also what surfaces a panic.
    Finished,
    /// The command could not be queued, or no acknowledgement arrived, within
    /// `flush_timeout`. The worker is busy on something whose length
    /// `flush_timeout` does not describe.
    TimedOut,
}

#[cfg(feature = "python")]
#[pymethods]
impl FemtoHTTPHandler {
    /// Parses the Python level, queues one record, and maps handler failures to `PyRuntimeError`.
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
    /// joined. One that does not, typically because it is still retrying a
    /// request against an unresponsive endpoint, is abandoned with a warning
    /// rather than waited for. It keeps working through the records already
    /// queued and then exits on its own, so those records are delivered only
    /// if the process outlives it.
    #[pyo3(name = "close")]
    fn py_close(&mut self) {
        self.close();
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

    fn flush_until(&self, deadline: Instant) -> bool {
        self.flush_by(deadline)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

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
