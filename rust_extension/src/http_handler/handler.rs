//! Public handler type exported by the crate.

use std::{thread, time::Duration};

#[cfg(feature = "python")]
use pyo3::prelude::*;

use parking_lot::Mutex;

use crate::{
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
    rate_limited_warner::RateLimitedWarner,
};

use super::{
    config::HTTPHandlerConfig,
    worker::{HTTPCommand, enqueue_record, flush_queue, spawn_worker},
};

#[cfg_attr(feature = "python", pyclass)]
/// Handler forwarding records to an HTTP endpoint.
///
/// Supports URL-encoded form data (CPython parity) and JSON serialization.
/// Uses exponential backoff for transient failures (5xx, 429, network errors)
/// and drops records on permanent failures (4xx except 429).
pub struct FemtoHTTPHandler {
    tx: Option<crossbeam_channel::Sender<HTTPCommand>>,
    handle: Mutex<Option<thread::JoinHandle<()>>>,
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
        <Self as FemtoHandlerTrait>::flush(self)
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

    fn sender(&self) -> Option<crossbeam_channel::Sender<HTTPCommand>> {
        self.tx.as_ref().cloned()
    }

    /// Ask the worker to stop, and report whether joining it is now bounded.
    ///
    /// The distinction the return value carries is the whole point: every
    /// outcome except a timed-out acknowledgement means the worker has left,
    /// or is leaving, its command loop, so the join that follows is short. An
    /// earlier version discarded this with `let _ =`, which is what let an
    /// unbounded join follow a bounded wait.
    fn request_shutdown(&mut self) -> ShutdownOutcome {
        let Some(tx) = self.tx.take() else {
            // Already closed. There is no worker left to wait for.
            return ShutdownOutcome::Finished;
        };
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        if tx.send(HTTPCommand::Shutdown(ack_tx)).is_err() {
            // The worker has dropped its receiver, so it has already left the
            // loop; joining it returns at once and may report a panic.
            return ShutdownOutcome::Finished;
        }
        match ack_rx.recv_timeout(self.flush_timeout) {
            Ok(()) => ShutdownOutcome::Finished,
            // The worker dropped the acknowledgement channel without using it,
            // which it can only do on its way out.
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => ShutdownOutcome::Finished,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => ShutdownOutcome::TimedOut,
        }
    }

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
    /// No acknowledgement arrived within `flush_timeout`. The worker is busy
    /// on something whose length `flush_timeout` does not describe.
    TimedOut,
}

#[cfg(feature = "python")]
#[pymethods]
impl FemtoHTTPHandler {
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
    fn py_flush(&self) -> bool {
        self.flush()
    }

    /// Close the handler and wait for the worker thread to finish.
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
        let Some(tx) = self.sender() else {
            return false;
        };
        self.warner.flush(|count| {
            log::warn!("FemtoHTTPHandler dropped {count} records in the last interval");
        });
        flush_queue(&tx, self.flush_timeout)
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
