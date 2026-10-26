//! Tests for rejecting handler filters on the logger producer path.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, bounded};
use rstest::rstest;
use serial_test::serial;

use super::logger_tests_helpers::CollectingHandler;
use super::*;
use crate::filters::{FemtoFilter, FilterContext, FilterDecision};
use crate::handler::HandlerError;
use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};
use crate::logger::dispatcher::Shard;

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
    let shard: &'static Shard = Box::leak(Box::new(
        Shard::start(0, 4).expect("a four-record test shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("producer".to_string(), shard);
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
        handler: Arc::new(CollectingHandler::new()),
        filters: vec![Arc::new(RejectingFilter)],
    });
    logger.dispatch_to_handlers(FemtoLogRecord::new(
        "producer",
        FemtoLevel::Info,
        "filtered-out",
    ));

    let (_, budget) = logger
        .dispatch_state()
        .expect("test logger's dispatcher should be available");
    assert_eq!(
        budget.in_flight(),
        1,
        "a rejected record must not reserve a dispatch permit",
    );
    release_tx
        .send(())
        .expect("blocked handler should receive its release signal");

    assert!(
        logger.flush_handlers(),
        "the blocked record should drain after release",
    );
}

/// A shard refusal increments the logger counter and emits its bounded warning.
#[test]
#[serial]
fn refused_shard_admission_counts_and_warns_about_the_drop() {
    install_test_logger();
    let shard: &'static Shard = Box::leak(Box::new(
        Shard::start(0, 1).expect("a one-record test shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("producer-drop".to_string(), shard);
    let (started_tx, started_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    logger
        .add_handler(Arc::new(BlockingHandler {
            started: started_tx,
            release: release_rx,
        }))
        .expect("native held handler should register");

    assert!(
        logger
            .log(FemtoLevel::Info, "occupies the only permit")
            .is_some(),
        "the first record should be accepted for dispatch",
    );
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the worker should start handling the first record");

    assert!(
        logger.log(FemtoLevel::Info, "cannot fit").is_some(),
        "the public logging call still formats a refused record",
    );
    assert_eq!(
        logger.get_dropped(),
        1,
        "a shard refusal must reach the logger's producer drop counter",
    );
    let warnings = take_logged_messages();
    let capacity_warnings = warnings
        .iter()
        .filter(|warning| {
            warning.level == log::Level::Warn
                && warning.message == "FemtoLogger: dropped 1 records; dispatch capacity exhausted"
        })
        .count();
    assert_eq!(
        capacity_warnings, 1,
        "the refused record must produce one rate-limited capacity warning: {warnings:?}",
    );

    release_tx
        .send(())
        .expect("the worker should still be waiting for its first record");
    assert!(
        logger.flush_handlers(),
        "the accepted record should drain after the handler is released",
    );
}
