//! Outcome types for the file handler's enqueue, flush, and shutdown paths.
//!
//! The file handler distinguishes more results than its public `bool` and
//! `()` returns can express: an enqueue may be refused because the queue is
//! full, because a deadline expired, because the handler was closed, or
//! because the worker's channel disappeared; a flush may fail at the enqueue
//! or at the acknowledgement; and a close wait may complete, time out, join a
//! panicked worker, or be unable to establish a verdict at all. These enums
//! keep that detail available to the Python bindings, which map each variant
//! onto a bounded wait diagnostic.
//!
//! [`WorkerShutdown`] also lives here so the bindings can take the shutdown
//! state under a short borrow and then wait without holding it.

use std::{sync::Arc, time::Duration};

use crate::handler::HandlerError;
use crate::handlers::telemetry::WaitOutcome;
use crate::shutdown::{ShutdownCompletion, TerminalStatus, WorkerTerminal};
use crate::sync::JoinHandle;

/// Why one enqueue attempt failed, before the public error mapping.
///
/// `HandlerError` collapses a deliberate close and a lost worker into one
/// `Closed` variant. The diagnostics need them apart: a post-close call is
/// expected, while a channel that vanished under an open handler is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnqueueFailure {
    /// The queue was full and the policy refused to wait.
    QueueFull,
    /// The policy's deadline expired before the worker accepted the record.
    TimedOut(Duration),
    /// The handler had already begun closing.
    Closed,
    /// The worker's channel disappeared while the handler was still open.
    Disconnected,
}

impl EnqueueFailure {
    /// Map to the public error the handler trait reports.
    ///
    /// `Closed` and `Disconnected` both become [`HandlerError::Closed`]: the
    /// public contract has always reported a lost worker as a closed handler,
    /// and the distinction exists only for the bounded diagnostics.
    pub(crate) fn into_handler_error(self) -> HandlerError {
        match self {
            Self::QueueFull => HandlerError::QueueFull,
            Self::TimedOut(duration) => HandlerError::Timeout(duration),
            Self::Closed | Self::Disconnected => HandlerError::Closed,
        }
    }
}

/// Owns the worker state needed to finish a handler shutdown after its Python
/// object borrow has been released.
pub(crate) struct WorkerShutdown {
    /// Worker thread handle retained for joining after completion.
    handle: Option<JoinHandle<()>>,
    /// Broadcast completion every closer waits on, so a closer that does not
    /// own the join handle still observes worker shutdown.
    completion: Arc<ShutdownCompletion>,
    /// Verdict the worker recorded as it left, used when this closer has no
    /// join handle to ask.
    terminal: Arc<TerminalStatus>,
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
    /// The worker signalled completion, but no closer could establish whether
    /// the exit was clean.
    Unknown,
}

impl WorkerShutdown {
    /// Pair an optional join handle with the shared completion signal and the
    /// worker's terminal verdict.
    pub(crate) fn new(
        handle: Option<JoinHandle<()>>,
        completion: Arc<ShutdownCompletion>,
        terminal: Arc<TerminalStatus>,
    ) -> Self {
        Self {
            handle,
            completion,
            terminal,
        }
    }

    /// Wait for worker completion and report how the worker left.
    ///
    /// Every caller waits on the shared completion, including one that finds
    /// no join handle left: another closer may still be joining a worker that
    /// has not finished writing its queued records. A closer without the
    /// handle reads the verdict the worker recorded before signalling
    /// completion, so a panic that happened after an earlier closer timed out
    /// is still reported rather than being mistaken for a clean exit.
    pub(crate) fn wait(self) -> ShutdownOutcome {
        if !self.completion.wait(Duration::from_secs(1)) {
            log::warn!("FemtoFileHandler: worker thread did not shut down within 1s");
            return ShutdownOutcome::TimedOut;
        }
        if let Some(handle) = self.handle {
            if handle.join().is_err() {
                log::warn!("FemtoFileHandler: worker thread panicked");
                return ShutdownOutcome::Panicked;
            }
            return ShutdownOutcome::Completed;
        }
        match self.terminal.get() {
            WorkerTerminal::Clean => ShutdownOutcome::Completed,
            WorkerTerminal::Panicked => ShutdownOutcome::Panicked,
            WorkerTerminal::Unknown => ShutdownOutcome::Unknown,
        }
    }
}

/// How a flush request finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlushOutcome {
    /// The worker acknowledged that it flushed successfully.
    Acknowledged,
    /// The flush command could not be enqueued before the deadline.
    EnqueueTimedOut,
    /// The command was enqueued, but the worker did not acknowledge in time.
    AckTimedOut,
    /// The handler was already closed.
    Closed,
    /// The worker's channel disappeared while the handler was still open.
    Disconnected,
    /// The worker answered the flush with an I/O failure.
    Failed,
}

impl From<FlushOutcome> for WaitOutcome {
    fn from(outcome: FlushOutcome) -> Self {
        match outcome {
            FlushOutcome::Acknowledged => Self::FlushAcknowledged,
            FlushOutcome::EnqueueTimedOut => Self::FlushEnqueueTimedOut,
            FlushOutcome::AckTimedOut => Self::FlushAckTimedOut,
            FlushOutcome::Closed => Self::FlushClosed,
            FlushOutcome::Disconnected => Self::FlushDisconnected,
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
            ShutdownOutcome::Unknown => Self::CloseUnknown,
        }
    }
}

/// Classify an enqueue result for the wait diagnostics.
///
/// The four failures are kept apart because they call for different operator
/// responses: a full queue is back-pressure, a timeout is a slow worker, a
/// closed handler is an expected post-shutdown call, and a disconnected
/// channel is a worker that died while the handler still looked open.
pub(crate) fn classify_enqueue(result: &Result<(), EnqueueFailure>) -> WaitOutcome {
    match result {
        Ok(()) => WaitOutcome::HandleAccepted,
        Err(EnqueueFailure::QueueFull) => WaitOutcome::HandleQueueFull,
        Err(EnqueueFailure::TimedOut(_)) => WaitOutcome::HandleTimedOut,
        Err(EnqueueFailure::Closed) => WaitOutcome::HandleClosed,
        Err(EnqueueFailure::Disconnected) => WaitOutcome::HandleDisconnected,
    }
}
