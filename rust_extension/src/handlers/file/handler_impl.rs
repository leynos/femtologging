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

impl FemtoHandlerTrait for FemtoFileHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let Some(tx) = &self.tx else {
            log::warn!("FemtoFileHandler: handle called after close");
            return Err(HandlerError::Closed);
        };
        match self.overflow_policy {
            OverflowPolicy::Drop => match tx.try_send(FileCommand::Record(Box::new(record))) {
                Ok(()) => Ok(()),
                Err(TrySendError::Full(_)) => {
                    log::warn!(
                        "FemtoFileHandler (Drop): queue full or shutting down, dropping record"
                    );
                    Err(HandlerError::QueueFull)
                }
                Err(TrySendError::Disconnected(_)) => {
                    log::warn!("FemtoFileHandler (Drop): queue closed, dropping record");
                    Err(HandlerError::Closed)
                }
            },
            OverflowPolicy::Block => match tx.send(FileCommand::Record(Box::new(record))) {
                Ok(()) => Ok(()),
                Err(_) => {
                    log::warn!(
                        "FemtoFileHandler (Block): queue full or shutting down, dropping record"
                    );
                    Err(HandlerError::Closed)
                }
            },
            OverflowPolicy::Timeout(dur) => {
                match tx.send_timeout(FileCommand::Record(Box::new(record)), dur) {
                    Ok(()) => Ok(()),
                    Err(SendTimeoutError::Timeout(_)) => {
                        log::warn!(
                            "FemtoFileHandler (Timeout): timed out waiting for queue, dropping record"
                        );
                        Err(HandlerError::Timeout(dur))
                    }
                    Err(SendTimeoutError::Disconnected(_)) => {
                        log::warn!("FemtoFileHandler (Timeout): queue closed, dropping record");
                        Err(HandlerError::Closed)
                    }
                }
            }
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
