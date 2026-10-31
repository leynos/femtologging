//! `FemtoHandlerTrait` and drop implementations for the stream handler.

use std::any::Any;

use log::warn;

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;
use crate::sync::TrySendError;

use super::{FemtoStreamHandler, StreamCommand};

impl FemtoHandlerTrait for FemtoStreamHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let Some(tx) = &self.tx else {
            self.warner.record_drop();
            self.warner.warn_if_due(|count| {
                warn!("FemtoStreamHandler: {count} log records dropped in the last interval");
            });
            return Err(HandlerError::Closed);
        };
        match tx.try_send(StreamCommand::Record(record)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.warner.record_drop();
                self.warner.warn_if_due(|count| {
                    warn!("FemtoStreamHandler: {count} log records dropped in the last interval");
                });
                Err(HandlerError::QueueFull)
            }
            Err(TrySendError::Disconnected(_)) => {
                self.warner.record_drop();
                self.warner.warn_if_due(|count| {
                    warn!("FemtoStreamHandler: {count} log records dropped in the last interval");
                });
                Err(HandlerError::Closed)
            }
        }
    }

    fn flush(&self) -> bool {
        FemtoStreamHandler::flush(self)
    }

    fn flush_until(&self, deadline: std::time::Instant) -> bool {
        self.flush_by(deadline)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for FemtoStreamHandler {
    fn drop(&mut self) {
        self.close();
    }
}
