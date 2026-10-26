//! Stream handler shutdown logic detached from Python borrows.

use std::time::Duration;

use log::warn;

use super::StreamCommand;
use crate::sync::{JoinHandle, Receiver, Sender};

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
