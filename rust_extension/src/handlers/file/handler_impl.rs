//! Trait and lifecycle implementations for `FemtoFileHandler`.
//!
//! Splitting these impl blocks out of `mod.rs` keeps the main handler module
//! focused on construction and configuration while preserving the same public
//! API.

use std::any::Any;

use crossbeam_channel::{SendTimeoutError, TrySendError};

use super::{FemtoFileHandler, FileCommand, OverflowPolicy};
use crate::{
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
};

/// Log a failed enqueue and surface the corresponding handler error.
fn queue_failure(policy: &str, reason: &str, err: HandlerError) -> HandlerError {
    log::warn!("FemtoFileHandler ({policy}): {reason}");
    err
}

/// Enqueue a record, blocking until the worker accepts it.
fn send_blocking(
    tx: &crossbeam_channel::Sender<FileCommand>,
    command: FileCommand,
) -> Result<(), HandlerError> {
    tx.send(command).map_err(|_| {
        queue_failure(
            "Block",
            "channel disconnected or shutting down",
            HandlerError::Closed,
        )
    })
}

/// Enqueue a record via `try_send` (Drop) or `send_timeout` (Timeout).
///
/// Both channel APIs share one failure taxonomy — the queue is saturated or
/// the channel is closed — so the mapping lives in a single place.
fn send_bounded(
    tx: &crossbeam_channel::Sender<FileCommand>,
    command: FileCommand,
    timeout: Option<std::time::Duration>,
) -> Result<(), HandlerError> {
    let (policy, saturation_reason, saturation_error, outcome) = match timeout {
        Some(dur) => (
            "Timeout",
            "timed out waiting for queue, dropping record",
            HandlerError::Timeout(dur),
            tx.send_timeout(command, dur)
                .map_err(|err| matches!(err, SendTimeoutError::Disconnected(_))),
        ),
        None => (
            "Drop",
            "queue full, dropping record",
            HandlerError::QueueFull,
            tx.try_send(command)
                .map_err(|err| matches!(err, TrySendError::Disconnected(_))),
        ),
    };
    outcome.map_err(|is_disconnected| {
        if is_disconnected {
            queue_failure(
                policy,
                "queue closed, dropping record",
                HandlerError::Closed,
            )
        } else {
            queue_failure(policy, saturation_reason, saturation_error)
        }
    })
}

impl FemtoHandlerTrait for FemtoFileHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let Some(tx) = &self.tx else {
            log::warn!("FemtoFileHandler: handle called after close");
            return Err(HandlerError::Closed);
        };
        let command = FileCommand::Record(Box::new(record));
        match self.overflow_policy {
            OverflowPolicy::Drop => send_bounded(tx, command, None),
            OverflowPolicy::Block => send_blocking(tx, command),
            OverflowPolicy::Timeout(dur) => send_bounded(tx, command, Some(dur)),
        }
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
