//! Stream handler shutdown logic detached from Python borrows.

use std::time::Duration;

use log::warn;

use super::{FemtoStreamHandler, StreamCommand};
use crate::sync::{JoinHandle, Receiver, Sender};

/// Worker resources moved out of the Python handler before shutdown waits.
pub(super) struct StreamShutdownState {
    /// Sender whose removal stops the worker after in-flight commands finish.
    tx: Option<Sender<StreamCommand>>,
    /// Join handle consumed by the first close operation.
    handle: Option<JoinHandle<()>>,
    /// Completion signal used to enforce the configured shutdown timeout.
    done_rx: Receiver<()>,
    /// Maximum time to wait for worker completion.
    timeout: Duration,
}

impl StreamShutdownState {
    /// Wait for worker completion and join it when it exits before the timeout.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// shutdown.close();
    /// ```
    pub(super) fn close(self) {
        close_stream_worker(self.tx, self.handle, self.done_rx, self.timeout);
    }
}

impl FemtoStreamHandler {
    /// Take worker ownership under short locks shared by Python methods.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let shutdown = handler.take_shutdown_state();
    /// shutdown.close();
    /// ```
    pub(super) fn take_shutdown_state(&self) -> StreamShutdownState {
        StreamShutdownState {
            tx: self.tx.lock().take(),
            handle: self.handle.lock().take(),
            done_rx: self.done_rx.lock().clone(),
            timeout: self.flush_timeout,
        }
    }
}

/// Drops the queue sender, then waits for and joins the worker without a Python borrow.
///
/// # Examples
///
/// ```ignore
/// close_stream_worker(sender, handle, worker_done_receiver, timeout);
/// ```
pub(super) fn close_stream_worker(
    tx: Option<Sender<StreamCommand>>,
    handle: Option<JoinHandle<()>>,
    done_rx: Receiver<()>,
    timeout: Duration,
) {
    drop(tx);
    if let Some(handle) = handle {
        if done_rx.recv_timeout(timeout).is_err() {
            warn!(
                "FemtoStreamHandler: worker thread did not shut down within {:?}",
                timeout
            );
            return;
        }
        if handle.join().is_err() {
            warn!("FemtoStreamHandler: worker thread panicked");
        }
    }
}
