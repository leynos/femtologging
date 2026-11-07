//! Owned worker state for closing a stream handler outside its Python borrow.
//!
//! `FemtoStreamHandler::close` may run from `Drop`, which a Python caller can
//! trigger while still holding the GIL. PyO3 attachment can be unavailable
//! during interpreter shutdown or GC traversal, so the drop decision lives
//! here beside the shutdown operation it drives.

use std::{sync::Arc, time::Duration};

use log::warn;
use pyo3::Python;

use super::FemtoStreamHandler;
use crate::{shutdown::ShutdownCompletion, sync::JoinHandle};

impl Drop for FemtoStreamHandler {
    fn drop(&mut self) {
        if self.handle.lock().is_none() {
            return;
        }
        // Release the GIL before waiting: a worker still formatting a record
        // can call back into Python, which needs the GIL this thread holds.
        // `try_attach` runs the closure when the thread is already attached,
        // and returns `None` only when the interpreter cannot be attached to:
        // before initialization, while finalizing, or during GC traversal.
        // Those are exactly the cases with no GIL to release, and closing
        // directly stays bounded because its wait is on a timeout.
        if Python::try_attach(|py| py.detach(|| self.close())).is_none() {
            self.close();
        }
    }
}

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
