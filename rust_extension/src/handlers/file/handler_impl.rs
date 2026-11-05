//! Trait and lifecycle implementations for `FemtoFileHandler`.
//!
//! Splitting these impl blocks out of `mod.rs` keeps the main handler module
//! focused on construction and configuration while preserving the same public
//! API.

use std::any::Any;

use super::{EnqueueFailure, FemtoFileHandler, FileCommand, OverflowPolicy};
use crate::{
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
    sync::{SendTimeoutError, Sender, TrySendError},
};

/// Log a failed enqueue and surface the corresponding failure.
fn queue_failure(policy: &str, reason: &str, failure: EnqueueFailure) -> EnqueueFailure {
    log::warn!("FemtoFileHandler ({policy}): {reason}");
    failure
}

/// Enqueue a record, blocking until the worker accepts it.
fn send_blocking(tx: &Sender<FileCommand>, command: FileCommand) -> Result<(), EnqueueFailure> {
    tx.send(command).map_err(|_| {
        queue_failure(
            "Block",
            "channel disconnected or shutting down",
            EnqueueFailure::Disconnected,
        )
    })
}

/// Enqueue a record via `try_send` (Drop) or `send_timeout` (Timeout).
///
/// Both channel APIs share one failure taxonomy — the queue is saturated or
/// the channel is closed — so the mapping lives in a single place.
fn send_bounded(
    tx: &Sender<FileCommand>,
    command: FileCommand,
    timeout: Option<std::time::Duration>,
) -> Result<(), EnqueueFailure> {
    let (policy, saturation_reason, saturation_failure, outcome) = match timeout {
        Some(dur) => (
            "Timeout",
            "timed out waiting for queue, dropping record",
            EnqueueFailure::TimedOut(dur),
            tx.send_timeout(command, dur)
                .map_err(|err| matches!(err, SendTimeoutError::Disconnected(_))),
        ),
        None => (
            "Drop",
            "queue full, dropping record",
            EnqueueFailure::QueueFull,
            tx.try_send(command)
                .map_err(|err| matches!(err, TrySendError::Disconnected(_))),
        ),
    };
    outcome.map_err(|is_disconnected| {
        if is_disconnected {
            queue_failure(
                policy,
                "queue closed, dropping record",
                EnqueueFailure::Disconnected,
            )
        } else {
            queue_failure(policy, saturation_reason, saturation_failure)
        }
    })
}

impl FemtoFileHandler {
    /// Enqueue one record and report why the attempt failed, if it did.
    ///
    /// The result is richer than [`HandlerError`] so the Python bindings can
    /// tell a deliberate close from a worker whose channel disappeared. The
    /// public trait method maps both onto `HandlerError::Closed`.
    pub(crate) fn enqueue(&self, record: FemtoLogRecord) -> Result<(), EnqueueFailure> {
        // Clone under the lock and release it before sending: a blocking send
        // must not hold the lock that close needs to take the sender.
        let Some(tx) = self.tx.lock().clone() else {
            log::warn!("FemtoFileHandler: handle called after close");
            return Err(EnqueueFailure::Closed);
        };
        let command = FileCommand::Record(Box::new(record));
        match self.overflow_policy {
            OverflowPolicy::Drop => send_bounded(&tx, command, None),
            OverflowPolicy::Block => send_blocking(&tx, command),
            OverflowPolicy::Timeout(dur) => send_bounded(&tx, command, Some(dur)),
        }
    }
}

impl FemtoHandlerTrait for FemtoFileHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.enqueue(record)
            .map_err(EnqueueFailure::into_handler_error)
    }

    fn flush(&self) -> bool {
        FemtoFileHandler::flush(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for FemtoFileHandler {
    fn drop(&mut self) {
        self.close();
    }
}
