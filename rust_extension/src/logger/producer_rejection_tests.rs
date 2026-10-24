//! Tests for rejecting handler filters on the logger producer path.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded};
use rstest::rstest;

use super::logger_tests_helpers::CountingHandler;
use super::*;
use crate::filters::{FemtoFilter, FilterContext, FilterDecision};
use crate::handler::HandlerError;

/// Filter fixture that rejects every record without enriching it.
struct RejectingFilter;

impl FemtoFilter for RejectingFilter {
    fn decision(
        &self,
        _record: &mut FemtoLogRecord,
        _context: &mut FilterContext,
    ) -> FilterDecision {
        FilterDecision {
            accepted: false,
            callback_failed: false,
            enrichment: BTreeMap::new(),
        }
    }
}

/// Handler that pauses worker delivery until the producer assertion completes.
struct BlockingHandler {
    /// Notifies the test that the worker has entered handler execution.
    started: Sender<()>,
    /// Keeps the worker blocked until the test releases the queued record.
    release: Receiver<()>,
}

impl FemtoHandlerTrait for BlockingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        self.started.send(()).map_err(|_| HandlerError::Closed)?;
        self.release.recv().map_err(|_| HandlerError::Closed)?;
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[rstest]
fn rejected_handler_filters_do_not_enqueue_empty_jobs() {
    let logger = FemtoLogger::new("producer".to_string());
    let tx = logger
        .tx
        .as_ref()
        .expect("logger should have a queue")
        .clone();
    let (started_tx, started_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    logger
        .handlers
        .write()
        .push(HandlerAttachment::new(Arc::new(BlockingHandler {
            started: started_tx,
            release: release_rx,
        })));

    logger.dispatch_to_handlers(FemtoLogRecord::new(
        "producer",
        FemtoLevel::Info,
        "block-worker",
    ));
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("worker should be blocked inside the first handler");

    logger.handlers.write().clear();
    logger.handlers.write().push(HandlerAttachment {
        handler: Arc::new(CountingHandler::default()),
        filters: vec![Arc::new(RejectingFilter)],
    });
    logger.dispatch_to_handlers(FemtoLogRecord::new(
        "producer",
        FemtoLevel::Info,
        "filtered-out",
    ));

    let queued_jobs = tx.len();
    release_tx
        .send(())
        .expect("blocked handler should receive its release signal");

    assert_eq!(queued_jobs, 0, "rejected records must not occupy the queue");
}
