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

/// Enqueue a record without blocking, dropping it when the queue is full.
fn send_dropping(
    tx: &crossbeam_channel::Sender<FileCommand>,
    command: FileCommand,
) -> Result<(), HandlerError> {
    match tx.try_send(command) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            log::warn!("FemtoFileHandler (Drop): queue full, dropping record");
            Err(HandlerError::QueueFull)
        }
        Err(TrySendError::Disconnected(_)) => {
            log::warn!("FemtoFileHandler (Drop): queue closed, dropping record");
            Err(HandlerError::Closed)
        }
    }
}

/// Enqueue a record, blocking until the worker accepts it.
fn send_blocking(
    tx: &crossbeam_channel::Sender<FileCommand>,
    command: FileCommand,
) -> Result<(), HandlerError> {
    match tx.send(command) {
        Ok(()) => Ok(()),
        Err(_) => {
            log::warn!("FemtoFileHandler (Block): channel disconnected or shutting down");
            Err(HandlerError::Closed)
        }
    }
}

/// Enqueue a record, giving up after the configured timeout elapses.
fn send_with_timeout(
    tx: &crossbeam_channel::Sender<FileCommand>,
    command: FileCommand,
    dur: std::time::Duration,
) -> Result<(), HandlerError> {
    match tx.send_timeout(command, dur) {
        Ok(()) => Ok(()),
        Err(SendTimeoutError::Timeout(_)) => {
            log::warn!("FemtoFileHandler (Timeout): timed out waiting for queue, dropping record");
            Err(HandlerError::Timeout(dur))
        }
        Err(SendTimeoutError::Disconnected(_)) => {
            log::warn!("FemtoFileHandler (Timeout): queue closed, dropping record");
            Err(HandlerError::Closed)
        }
    }
}

impl FemtoHandlerTrait for FemtoFileHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let Some(tx) = &self.tx else {
            log::warn!("FemtoFileHandler: handle called after close");
            return Err(HandlerError::Closed);
        };
        let command = FileCommand::Record(Box::new(record));
        match self.overflow_policy {
            OverflowPolicy::Drop => send_dropping(tx, command),
            OverflowPolicy::Block => send_blocking(tx, command),
            OverflowPolicy::Timeout(dur) => send_with_timeout(tx, command, dur),
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
