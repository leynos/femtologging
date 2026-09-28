//! Queue payloads and handler attachments used by logger dispatch.

use std::{any::Any, sync::Arc};

use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::{
    filters::FemtoFilter,
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
};

/// Handler used internally to acknowledge logger flush operations.
pub(crate) struct FlushAckHandler {
    ack: Sender<()>,
}

impl FlushAckHandler {
    pub(crate) fn new(ack: Sender<()>) -> Self {
        Self { ack }
    }
}

impl FemtoHandlerTrait for FlushAckHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        let _ = self.ack.send(());
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Deliver one handler's producer-filter snapshot through the existing queue
/// payload without changing the public [`QueuedRecord`] shape.
pub(crate) struct HandlerRecordSnapshot {
    handler: Arc<dyn FemtoHandlerTrait>,
    record: Mutex<Option<FemtoLogRecord>>,
}

impl HandlerRecordSnapshot {
    pub(crate) fn snapshot_handler(
        handler: Arc<dyn FemtoHandlerTrait>,
        record: FemtoLogRecord,
    ) -> Arc<dyn FemtoHandlerTrait> {
        Arc::new(Self {
            handler,
            record: Mutex::new(Some(record)),
        })
    }
}

impl FemtoHandlerTrait for HandlerRecordSnapshot {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let record = self.record.lock().take().unwrap_or(record);
        self.handler.handle(record)
    }

    fn flush(&self) -> bool {
        self.handler.flush()
    }

    fn as_any(&self) -> &dyn Any {
        self.handler.as_any()
    }
}

/// Record queued for processing by the worker thread.
pub struct QueuedRecord {
    pub record: FemtoLogRecord,
    pub handlers: Vec<Arc<dyn FemtoHandlerTrait>>,
}

/// A handler paired with filters evaluated before work reaches its worker.
#[derive(Clone)]
pub(crate) struct HandlerAttachment {
    pub(crate) handler: Arc<dyn FemtoHandlerTrait>,
    pub(crate) filters: Vec<Arc<dyn FemtoFilter>>,
}

impl HandlerAttachment {
    pub(crate) fn new(handler: Arc<dyn FemtoHandlerTrait>) -> Self {
        Self {
            handler,
            filters: Vec::new(),
        }
    }

    #[cfg(feature = "python")]
    pub(crate) fn with_filters(
        handler: Arc<dyn FemtoHandlerTrait>,
        filters: Vec<Arc<dyn FemtoFilter>>,
    ) -> Self {
        Self { handler, filters }
    }
}
