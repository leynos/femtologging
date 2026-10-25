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
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
    rate_limited_warner::RateLimitedWarner,
    shutdown_signal::ShutdownSignal,
};

use super::{
    SocketTransport,
    config::SocketHandlerConfig,
    worker::{SocketCommand, enqueue_record, flush_queue, spawn_worker},
};

#[cfg_attr(feature = "python", pyclass)]
/// Handler forwarding records to a socket using MessagePack framing.
pub struct FemtoSocketHandler {
    /// Sends records and lifecycle commands to the worker.
    tx: Option<crossbeam_channel::Sender<SocketCommand>>,
    /// Owns the worker until it finishes or is detached after a timeout.
    handle: Mutex<Option<thread::JoinHandle<()>>>,
    /// Counts dropped records and limits warning frequency.
    warner: RateLimitedWarner,
    /// The shutdown signal shared with the worker.
    ///
    /// `Some` for as long as the handler is open. `None` once `close` has
    /// given up on the worker, or after a close that had nothing to do.
    signal: Option<Arc<ShutdownSignal>>,
    /// Budget shared by flush and shutdown waits.
    flush_timeout: Duration,
}

impl FemtoSocketHandler {
    /// Construct a handler targeting the provided transport with default configuration.
    pub fn new(transport: SocketTransport) -> Self {
        Self::with_config(SocketHandlerConfig::default().with_transport(transport))
    }

    /// Construct the handler from a configuration object.
    pub fn with_config(config: SocketHandlerConfig) -> Self {
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
        <Self as FemtoHandlerTrait>::flush(self)
    }

    /// Close the handler, waiting for the worker only while it is answering.
    ///
    /// One deadline covers the whole operation: the shutdown signal, the
    /// command that carries the acknowledgement request, the acknowledgement
    /// itself, and the join. A worker that does not acknowledge within that
    /// budget is abandoned rather than joined, because the alternative is to
    /// wait out whatever it is doing - a connect, a write, or a backoff sleep
    /// whose length the configuration describes separately and the caller did
    /// not ask for. `close` runs from `Drop`, so that wait would fall on
    /// whatever was tearing the handler down.
    ///
    /// The signal is what makes the budget apply to the work and not only to
    /// the waiting. Until it is raised the worker is free to retry a failed
    /// connection for as long as its backoff allows; once it is, no new attempt
    /// begins after the deadline, each queued record gets one attempt, and
    /// whatever is still queued at the deadline is discarded with a warning
    /// rather than written.
    pub fn close(&mut self) {
        match self.request_shutdown() {
            ShutdownOutcome::Finished => self.join_worker(),
            ShutdownOutcome::TimedOut => self.abandon_worker(),
        }
    }

    /// Clone the live worker sender, if the handler has not closed.
    ///
    /// Example: a missing sender makes `handle` return `HandlerError::Closed`.
    fn sender(&self) -> Option<crossbeam_channel::Sender<SocketCommand>> {
        self.tx.as_ref().cloned()
    }

    /// Ask the worker to stop, and report whether joining it is now bounded.
    ///
    /// Every outcome except a timeout means the worker has left, or is leaving,
    /// its command loop, so the join that follows is short.
    ///
    /// One deadline covers both halves. The command channel is bounded, and a
    /// plain `send` on a full one blocks until the worker next reads it - which
    /// a worker stuck mid-write does not do. Bounding the acknowledgement alone
    /// left that wait in front of it, and bounding neither left the caller
    /// waiting on a worker that might never answer at all.
    ///
    /// The signal is raised before the send, so it is the signal that wakes a
    /// worker sleeping in a backoff delay rather than the command that queues
    /// behind a full channel.
    fn request_shutdown(&mut self) -> ShutdownOutcome {
        let Some(tx) = self.tx.take() else {
            // Already closed. There is no worker left to wait for.
            return ShutdownOutcome::Finished;
        };
        let deadline = Instant::now() + self.flush_timeout;
        if let Some(signal) = self.signal.as_ref() {
            signal.begin_shutdown(deadline);
        }
        let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
        match tx.send_deadline(SocketCommand::Shutdown(ack_tx), deadline) {
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

    /// Ask to be told when the worker next begins a backoff delay.
    ///
    /// Exposed for tests, and only compiled for them. A test that asserts a
    /// close is observed *during* a retry delay has to know the worker reached
    /// that delay first; the alternative is a sleep long enough to guess it,
    /// which measures the host rather than the handler.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let backoffs = handler.wait_notification();
    /// handler.handle(record).expect("queue the record");
    /// backoffs.recv_timeout(one_second).expect("the worker should be backing off");
    /// ```
    #[cfg(test)]
    pub(crate) fn wait_notification(&self) -> std::sync::mpsc::Receiver<()> {
        self.signal
            .as_ref()
            .expect("an open handler has a signal")
            .notify_on_wait()
    }

    /// Pause the worker at its next backoff wait until the test resumes it.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let (started, resume) = handler.pause_next_wait();
    /// started.recv().expect("the worker reached backoff");
    /// resume.send(()).expect("resume the worker");
    /// ```
    #[cfg(test)]
    pub(crate) fn pause_next_wait(
        &self,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        self.signal
            .as_ref()
            .expect("an open handler has a signal")
            .pause_next_wait()
    }

    /// Clone the worker signal so a test can observe shutdown before releasing
    /// a worker paused at the backoff boundary.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let signal = handler.signal_for_test();
    /// assert!(!signal.is_shutting_down());
    /// ```
    #[cfg(test)]
    pub(crate) fn signal_for_test(&self) -> Arc<ShutdownSignal> {
        Arc::clone(self.signal.as_ref().expect("an open handler has a signal"))
    }

    /// Join the worker after its acknowledgement or channel disconnection.
    ///
    /// Example: an acknowledged shutdown takes and joins the worker handle.
    fn join_worker(&mut self) {
        self.signal.take();
        let Some(handle) = self.handle.lock().take() else {
            return;
        };
        if handle.join().is_err() {
            log::warn!("FemtoSocketHandler: worker thread panicked");
        }
    }

    /// Detach the worker after it failed to acknowledge shutdown.
    ///
    /// The handle is taken so nothing joins it later, in particular the `Drop`
    /// that may follow this call. The thread itself ends on its own once its
    /// in-flight write finishes, because the signal now says nothing further
    /// may be started and its command channel is already disconnected.
    ///
    /// The signal is aborted rather than simply dropped: the worker is the only
    /// holder of the other reference, and a signal left in the draining phase
    /// would keep telling it there was budget left until the drain deadline it
    /// was already past.
    fn abandon_worker(&mut self) {
        if let Some(signal) = self.signal.take() {
            signal.abort();
        }
        log::warn!(
            "FemtoSocketHandler: worker did not acknowledge shutdown within {:?}; \
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
impl FemtoSocketHandler {
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
    /// handler (default: 1 second).
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
    /// joined. One that does not, typically because it is stuck writing to an
    /// unresponsive collector, is abandoned with a warning rather than waited
    /// for, and the records still queued are discarded rather than written.
    ///
    /// The wait runs with the GIL released, so a close that spends its budget
    /// waiting does not hold the interpreter while it does.
    #[pyo3(name = "close")]
    fn py_close(&mut self, py: Python<'_>) {
        py.detach(|| self.close());
    }
}

impl FemtoHandlerTrait for FemtoSocketHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let Some(tx) = self.sender() else {
            self.warner.record_drop();
            self.warner.warn_if_due(|count| {
                log::warn!("FemtoSocketHandler dropped {count} records after shutdown");
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
            log::warn!("FemtoSocketHandler dropped {count} records in the last interval");
        });
        flush_queue(&tx, self.flush_timeout)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Drop the handler, closing it with the GIL released.
///
/// A handler whose worker has already gone has nothing to close, and attaching
/// would initialize an interpreter to do nothing with it; the check keeps that
/// case out of the GIL's way entirely. The close itself is the same one
/// [`FemtoSocketHandler::close`] performs, so a handler dropped without an
/// explicit close is bounded by the same budget, and the join it may perform
/// happens with the interpreter available to a worker that wants it.
///
/// Under `--cfg loom` there is no interpreter to attach to: no model builds one
/// of these handlers, and attaching would initialize Python on a Loom
/// coroutine's small stack.
#[cfg(not(loom))]
impl Drop for FemtoSocketHandler {
    fn drop(&mut self) {
        if self.handle.lock().is_none() {
            return;
        }
        Python::attach(|py| py.detach(|| self.close()));
    }
}

#[cfg(loom)]
impl Drop for FemtoSocketHandler {
    fn drop(&mut self) {
        self.close();
    }
}

impl std::fmt::Debug for FemtoSocketHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FemtoSocketHandler")
            .field("flush_timeout", &self.flush_timeout)
            .finish()
    }
}
