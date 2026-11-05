//! Outcome types for the file handler's shutdown and wait paths.
//!
//! The file handler distinguishes more results than its public `bool` and
//! `()` returns can express: a flush may be acknowledged, time out, or find
//! the handler closed, and a close wait may complete, time out, or join a
//! panicked worker. These enums keep that detail available to the Python
//! bindings, which map each variant onto a bounded wait diagnostic.
//!
//! [`WorkerShutdown`] also lives here so the bindings can take the shutdown
//! state under a short borrow and then wait without holding it.

use std::{sync::Arc, time::Duration};

use crate::handler::HandlerError;
use crate::handlers::telemetry::WaitOutcome;
use crate::shutdown::ShutdownCompletion;
use crate::sync::JoinHandle;

/// Owns the worker state needed to finish a handler shutdown after its Python
/// object borrow has been released.
pub(crate) struct WorkerShutdown {
    /// Worker thread handle retained for joining after completion.
    handle: Option<JoinHandle<()>>,
    /// Broadcast completion every closer waits on, so a closer that does not
    /// own the join handle still observes worker shutdown.
    completion: Arc<ShutdownCompletion>,
}

/// How a bounded `close()` wait finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShutdownOutcome {
    /// The worker signalled completion and its thread joined cleanly.
    Completed,
    /// The worker did not signal completion within the one-second bound.
    TimedOut,
    /// The worker signalled completion, but its thread had panicked.
    Panicked,
}

impl WorkerShutdown {
    /// Pair an optional join handle with the shared completion signal.
    pub(crate) fn new(handle: Option<JoinHandle<()>>, completion: Arc<ShutdownCompletion>) -> Self {
        Self { handle, completion }
    }

    /// Wait for worker completion and join it when it reports a clean exit.
    ///
    /// Every caller waits on the shared completion, including one that finds
    /// no join handle left: another closer may still be joining a worker that
    /// has not finished writing its queued records.
    pub(crate) fn wait(self) -> ShutdownOutcome {
        if !self.completion.wait(Duration::from_secs(1)) {
            log::warn!("FemtoFileHandler: worker thread did not shut down within 1s");
            return ShutdownOutcome::TimedOut;
        }
        if let Some(handle) = self.handle
            && handle.join().is_err()
        {
            log::warn!("FemtoFileHandler: worker thread panicked");
            return ShutdownOutcome::Panicked;
        }
        ShutdownOutcome::Completed
    }
}

/// How a flush request finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlushOutcome {
    /// The worker acknowledged that it flushed successfully.
    Acknowledged,
    /// The worker did not acknowledge before the one-second deadline.
    TimedOut,
    /// The handler was already closed or its channel had disconnected.
    Closed,
    /// The worker answered the flush with an I/O failure.
    Failed,
}

impl From<FlushOutcome> for WaitOutcome {
    fn from(outcome: FlushOutcome) -> Self {
        match outcome {
            FlushOutcome::Acknowledged => Self::FlushAcknowledged,
            FlushOutcome::TimedOut => Self::FlushTimedOut,
            FlushOutcome::Closed => Self::FlushClosed,
            FlushOutcome::Failed => Self::FlushFailed,
        }
    }
}

impl From<ShutdownOutcome> for WaitOutcome {
    fn from(outcome: ShutdownOutcome) -> Self {
        match outcome {
            ShutdownOutcome::Completed => Self::CloseCompleted,
            ShutdownOutcome::TimedOut => Self::CloseTimedOut,
            ShutdownOutcome::Panicked => Self::ClosePanicked,
        }
    }
}

/// Classify a `handle()` result for the wait diagnostics.
///
/// `HandlerError` is wider than the three file handlers produce, so an
/// unexpected variant is reported as a generic failure rather than silently
/// counted as a successful send.
pub(crate) fn classify_handle(result: &Result<(), HandlerError>) -> WaitOutcome {
    match result {
        Ok(()) => WaitOutcome::HandleAccepted,
        Err(HandlerError::QueueFull) => WaitOutcome::HandleQueueFull,
        Err(HandlerError::Timeout(_)) => WaitOutcome::HandleTimedOut,
        Err(HandlerError::Closed) => WaitOutcome::HandleClosed,
        Err(_) => WaitOutcome::HandleFailed,
    }
}
