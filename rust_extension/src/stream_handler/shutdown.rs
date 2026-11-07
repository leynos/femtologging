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

    /// Waits for completion without joining the worker thread.
    ///
    /// Every caller waits on the shared completion, including one that finds
    /// no join handle left: another closer may still be joining, so the
    /// completion signal—not the join handle—is what orders queued writes
    /// before a closer returns.
    ///
    /// The handle is dropped rather than joined. A successful wait means the
    /// worker finished its body and dropped the writer and formatter, but not
    /// that the thread has exited: thread-local destructors run after the
    /// body returns, and `join` waits for them. A formatter that initialises a
    /// blocking thread-local destructor would then hold a closer past its
    /// configured timeout, and `JoinHandle::is_finished` cannot distinguish
    /// that window—it reports `true` once the thread function returns, while
    /// `join` continues to wait. Dropping the handle detaches the thread,
    /// which the operating system reclaims when it exits. The worker reports
    /// its own panics, so nothing is lost by abandoning the join result.
    pub(super) fn wait(self) {
        if !self.completion.wait(self.timeout) {
            warn!(
                "FemtoStreamHandler: worker thread did not shut down within {:?}",
                self.timeout
            );
        }
        drop(self.handle);
    }
}
