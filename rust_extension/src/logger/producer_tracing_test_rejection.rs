//! Test the tracing event emitted when native dispatch admission is refused.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::logger::{FemtoLogger, QueuedRecord, Shard};

pub(super) fn emit_rejected_native_dispatch() -> Result<(), String> {
    let shard = Box::leak(Box::new(
        Shard::start(0, 1).map_err(|error| error.to_string())?,
    ));
    let logger = FemtoLogger::assembled_for_test("trace-reject".to_owned(), shard);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let handler = Arc::new(BlockingNativeTestHandler {
        started: started_tx,
        release: parking_lot::Mutex::new(release_rx),
    });
    logger
        .add_handler(Arc::clone(&handler) as Arc<dyn FemtoHandlerTrait>)
        .map_err(|error| error.to_string())?;

    let (dispatch_shard, budget) = logger.dispatch_state().map_err(|error| error.to_string())?;
    let queued = Box::new(QueuedRecord {
        record: FemtoLogRecord::new("trace-reject", FemtoLevel::Info, "blocked-record"),
        handlers: vec![handler],
        #[cfg(feature = "python")]
        context: None,
    });
    if dispatch_shard.admit(queued, budget).is_err() {
        return Err("test record should occupy the one-slot dispatch shard".to_owned());
    }
    let handler_started = started_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    let record_accepted = logger
        .log(FemtoLevel::Info, "message-secret-not-traced")
        .is_some();
    let _ = release_tx.send(());
    if !handler_started {
        return Err("test handler should hold the dispatch shard".to_owned());
    }
    if !record_accepted {
        return Err("rejected dispatch should have accepted its producer record".to_owned());
    }
    if !logger.flush_handlers_blocking() {
        return Err("test handler should drain after it is released".to_owned());
    }
    drop(logger);
    Ok(())
}

/// Native handler that pauses worker delivery until the test releases it.
struct BlockingNativeTestHandler {
    /// Signals that the queued record reached this handler.
    started: Sender<()>,
    /// Keeps the one-slot shard occupied until the producer refusal is tested.
    release: parking_lot::Mutex<Receiver<()>>,
}

impl FemtoHandlerTrait for BlockingNativeTestHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.started.send(()).map_err(|_| HandlerError::Closed)?;
        self.release
            .lock()
            .recv()
            .map_err(|_| HandlerError::Closed)?;
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
