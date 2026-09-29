//! A handler that writes on the dispatching thread, for the topology models.
//!
//! Loom models at most five threads, the model's own included. A
//! `FemtoStreamHandler` brings a worker thread with it, so a topology with
//! three handlers, two loggers and a second logging thread needs seven, and
//! Loom refuses to start it. The topology models are about routing: which
//! handlers the logger's worker calls, and how often. `SinkHandler` answers
//! that question without a thread of its own by formatting each record straight
//! into a Loom-instrumented buffer when the logger's worker calls it.
//!
//! What it gives up is the handler's own queue and worker, and
//! `loom_stream_push_delivery` covers those against the real stream handler.

use std::any::Any;
use std::io::Write;

use loom::sync::{Arc, Mutex};

use _femtologging_rs::{
    DefaultFormatter, FemtoFormatter, FemtoHandlerTrait, FemtoLogRecord, HandlerError,
};

/// A handler that appends each formatted record to a shared Loom buffer.
pub struct SinkHandler {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl SinkHandler {
    /// Create a handler appending to `buffer`.
    pub fn new(buffer: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { buffer }
    }
}

impl FemtoHandlerTrait for SinkHandler {
    fn handle(&self, record: FemtoLogRecord) -> Result<(), HandlerError> {
        let line = DefaultFormatter.format(&record);
        let mut buffer = self
            .buffer
            .lock()
            .map_err(|_| HandlerError::Message("sink buffer poisoned".to_owned()))?;
        writeln!(buffer, "{line}").map_err(|err| HandlerError::Message(err.to_string()))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
