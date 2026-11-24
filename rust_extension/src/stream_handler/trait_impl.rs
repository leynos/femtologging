//! `FemtoHandlerTrait` implementation for the stream handler.
//!
//! The `Drop` implementation lives in `shutdown.rs`, beside the shutdown
//! operation it drives, because it has to release the GIL before waiting.

use std::any::Any;

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;
use crate::sync::TrySendError;

use super::{FemtoStreamHandler, StreamCommand, warning::warn_dropped};

#[cfg(test)]
#[path = "flush_deadline_tests.rs"]
mod flush_deadline_tests;

impl FemtoHandlerTrait for FemtoStreamHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        // Clone under the lock and release it before sending: a blocking send
        // must not hold the lock that close needs to take the sender.
        let Some(tx) = self.tx.lock().clone() else {
            self.warner.record_drop();
            self.warner.warn_if_due(warn_dropped);
            return Err(HandlerError::Closed);
        };
        match tx.try_send(StreamCommand::Record(record)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.warner.record_drop();
                self.warner.warn_if_due(warn_dropped);
                Err(HandlerError::QueueFull)
            }
            Err(TrySendError::Disconnected(_)) => {
                self.warner.record_drop();
                self.warner.warn_if_due(warn_dropped);
                Err(HandlerError::Closed)
            }
        }
    }

    fn flush(&self) -> bool {
        FemtoStreamHandler::flush(self)
    }

    fn flush_until(&self, context: &crate::handler::FlushContext) -> bool {
        self.flush_by(context)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
