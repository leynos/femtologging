//! Queue payloads and handler attachments used by logger dispatch.

use std::{any::Any, sync::Arc};

use crossbeam_channel::Sender;
use parking_lot::Mutex;
#[cfg(feature = "python")]
use pyo3::{Py, PyAny};

use crate::{
    filters::FemtoFilter,
    handler::{FemtoHandlerTrait, HandlerError},
    log_record::FemtoLogRecord,
};

/// Handler used internally to acknowledge logger flush operations.
pub(crate) struct FlushAckHandler {
    /// Signals that the worker has reached the flush barrier.
    ack: Sender<()>,
}

impl FlushAckHandler {
    /// Create an internal handler that acknowledges its next queued record.
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
    /// Original handler that receives the record after its producer filters run.
    handler: Arc<dyn FemtoHandlerTrait>,
    /// One-shot copy of the record state accepted by this handler's filters.
    record: Mutex<Option<FemtoLogRecord>>,
}

impl HandlerRecordSnapshot {
    /// Wrap a handler so its worker receives the producer-time record snapshot.
    pub(crate) fn snapshot_handler(
        handler: Arc<dyn FemtoHandlerTrait>,
        record: FemtoLogRecord,
    ) -> Arc<dyn FemtoHandlerTrait> {
        Arc::new(Self {
            handler,
            record: Mutex::new(Some(record)),
        })
    }

    /// Consume the saved snapshot once, falling back to the queue's shared record.
    fn take_record(&self, fallback: FemtoLogRecord) -> FemtoLogRecord {
        self.record.lock().take().unwrap_or(fallback)
    }
}

impl FemtoHandlerTrait for HandlerRecordSnapshot {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.handler.handle(self.take_record(record))
    }

    fn is_python_backed(&self) -> bool {
        self.handler.is_python_backed()
    }

    #[cfg(feature = "python")]
    fn provides_context_dispatch(&self) -> bool {
        self.handler.provides_context_dispatch()
    }

    #[cfg(feature = "python")]
    fn handle_with_context(
        &self,
        record: FemtoLogRecord,
        context: Option<&Py<PyAny>>,
    ) -> Result<(), HandlerError> {
        self.handler
            .handle_with_context(self.take_record(record), context)
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
    #[cfg(feature = "python")]
    pub context: Option<Py<PyAny>>,
}

/// A handler paired with filters evaluated before work reaches its worker.
#[derive(Clone)]
pub(crate) struct HandlerAttachment {
    /// Destination whose worker receives records approved by this attachment.
    pub(crate) handler: Arc<dyn FemtoHandlerTrait>,
    /// Ordered producer-side filters that can enrich or reject those records.
    pub(crate) filters: Vec<Arc<dyn FemtoFilter>>,
}

impl HandlerAttachment {
    /// Pair a handler with an empty filter chain for unfiltered attachments.
    pub(crate) fn new(handler: Arc<dyn FemtoHandlerTrait>) -> Self {
        Self {
            handler,
            filters: Vec::new(),
        }
    }

    /// Pair a handler with the ordered filters resolved from its configuration.
    #[cfg(feature = "python")]
    pub(crate) fn with_filters(
        handler: Arc<dyn FemtoHandlerTrait>,
        filters: Vec<Arc<dyn FemtoFilter>>,
    ) -> Self {
        Self { handler, filters }
    }
}
