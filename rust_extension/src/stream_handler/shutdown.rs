//! Owned worker state for closing a stream handler outside its Python borrow.

use std::time::Duration;

use log::warn;

use crate::sync::{JoinHandle, Receiver};

/// Waits for a stream worker after the handler has stopped accepting commands.
pub(super) struct StreamShutdown {
    /// Worker thread handle retained for joining after completion.
    handle: Option<JoinHandle<()>>,
    /// Completion signal used to bound the wait before joining.
    done_rx: Receiver<()>,
    /// Maximum time to wait for the worker's completion signal.
    timeout: Duration,
}

impl StreamShutdown {
    /// Captures all state needed to finish one handler shutdown.
    pub(super) fn new(
        handle: Option<JoinHandle<()>>,
        done_rx: Receiver<()>,
        timeout: Duration,
    ) -> Self {
        Self {
            handle,
            done_rx,
            timeout,
        }
    }

    /// Waits for completion and joins the worker when it reports a clean exit.
    pub(super) fn wait(self) {
        if let Some(handle) = self.handle {
            if self.done_rx.recv_timeout(self.timeout).is_err() {
                warn!(
                    "FemtoStreamHandler: worker thread did not shut down within {:?}",
                    self.timeout
                );
                return;
            }
            if handle.join().is_err() {
                warn!("FemtoStreamHandler: worker thread panicked");
            }
        }
    }
}
