//! Owned worker state for closing a stream handler outside its Python borrow.

use std::{sync::Arc, time::Duration};

use log::warn;

use crate::{shutdown::ShutdownCompletion, sync::JoinHandle};

/// Waits for a stream worker after the handler has stopped accepting commands.
pub(super) struct StreamShutdown {
    /// Worker thread handle retained for joining after completion.
    handle: Option<JoinHandle<()>>,
    /// Broadcast completion shared by every closer.
    completion: Arc<ShutdownCompletion>,
    /// Maximum time to wait for the worker's completion signal.
    timeout: Duration,
}

impl StreamShutdown {
    /// Captures all state needed to finish one handler shutdown.
    pub(super) fn new(
        handle: Option<JoinHandle<()>>,
        completion: Arc<ShutdownCompletion>,
        timeout: Duration,
    ) -> Self {
        Self {
            handle,
            completion,
            timeout,
        }
    }

    /// Waits for completion and joins the worker when it reports a clean exit.
    ///
    /// Every caller waits on the shared completion, including one that finds
    /// no join handle left: another closer may still be joining a worker that
    /// has not finished writing its queued records.
    ///
    /// The worker signals only after dropping its writer and formatter, so the
    /// join that follows a successful wait is bounded by thread teardown
    /// rather than by user code. When the timed wait expires no join is
    /// attempted, so a worker stuck in user code cannot extend the timeout.
    pub(super) fn wait(self) {
        if !self.completion.wait(self.timeout) {
            warn!(
                "FemtoStreamHandler: worker thread did not shut down within {:?}",
                self.timeout
            );
            return;
        }
        if let Some(handle) = self.handle
            && handle.join().is_err()
        {
            warn!("FemtoStreamHandler: worker thread panicked");
        }
    }
}
