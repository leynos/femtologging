//! Worker shutdown state for [`FemtoFileHandler`](super::FemtoFileHandler).
//!
//! A Python `close()` call must stop the worker without holding a borrow on the
//! handler object, so the state needed to finish shutdown is moved out of the
//! handler first. This module owns that moved state and the wait it performs.

use std::sync::Arc;
use std::time::Duration;

use crate::shutdown::ShutdownCompletion;
use crate::sync::JoinHandle;

/// Owns the worker state needed to finish a handler shutdown after its Python
/// object borrow has been released.
pub(in crate::handlers) struct WorkerShutdown {
    /// Worker thread handle retained for joining after completion.
    pub(super) handle: Option<JoinHandle<()>>,
    /// Broadcast completion every closer waits on, so a closer that does not
    /// own the join handle still observes worker shutdown.
    pub(super) completion: Arc<ShutdownCompletion>,
}

impl WorkerShutdown {
    /// Wait for worker completion and join it when it reports a clean exit.
    ///
    /// Every caller waits on the shared completion, including one that finds
    /// no join handle left: another closer may still be joining a worker that
    /// has not finished writing its queued records.
    pub(in crate::handlers) fn wait(self) {
        if !self.completion.wait(Duration::from_secs(1)) {
            log::warn!("FemtoFileHandler: worker thread did not shut down within 1s");
            return;
        }
        if let Some(handle) = self.handle
            && handle.join().is_err()
        {
            log::warn!("FemtoFileHandler: worker thread panicked");
        }
    }
}
