//! Tests for rejecting handler filters on the logger producer path.
//!
//! `RejectingFilter` implements `FemtoFilter` to reject every record, and
//! `BlockingHandler` implements `FemtoHandlerTrait` to hold the worker inside
//! `handle`. That lets `rejected_handler_filters_do_not_enqueue_empty_jobs`
//! assert no dispatch permit is reserved for a filtered record, while the
//! serialised `refused_shard_admission_counts_and_warns_about_the_drop` drives
//! a full `Shard` to show a refused record reaching `get_dropped` and emitting
//! one rate-limited capacity warning, while
//! `destruction_reports_drops_held_back_by_the_warning_interval` pins the
//! destruction report for a suppressed count.

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
use crate::handlers::file::test_support::{CapturedLog, install_test_logger, take_logged_messages};
use crate::logger::dispatcher::Shard;
use crate::rate_limited_warner::{Clock, DEFAULT_WARN_INTERVAL, RateLimitedWarner};

/// A clock that never advances, so a warning cannot become due by elapsed time.
struct FixedClock;

impl Clock for FixedClock {
    fn now_millis(&self) -> u64 {
        0
    }
}

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

    fn flush_until(&self, _context: &crate::handler::FlushContext) -> bool {
        true
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
    let logger = FemtoLogger::assembled_for_test("producer".to_owned(), shard);
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
    let logger = FemtoLogger::assembled_for_test("producer-drop".to_owned(), shard);
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

/// Count the captured warnings that carry exactly `message`.
///
/// The tests below assert both "reported once" and "not reported yet", so the
/// filter lives here rather than being repeated with a different expected
/// count; a mismatch prints the whole capture through the caller's message.
fn count_warnings(warnings: &[CapturedLog], message: &str) -> usize {
    warnings
        .iter()
        .filter(|warning| warning.level == log::Level::Warn && warning.message == message)
        .count()
}

/// Assemble a logger whose only permit is held by a blocked handler.
///
/// A one-record shard leaves room for exactly one accepted record, so later
/// records are refused at admission while the test decides what to do. The
/// drop warner takes a fixed clock: with the real one, a pause longer than the
/// warning interval would let a suppressed refusal report itself.
fn blocked_logger() -> (FemtoLogger, Receiver<()>, Sender<()>) {
    install_test_logger();
    let shard: &'static Shard = Box::leak(Box::new(
        Shard::start(0, 1).expect("a one-record test shard should start"),
    ));
    let mut logger = FemtoLogger::assembled_for_test("producer-flush".to_string(), shard);
    logger.drop_warner = RateLimitedWarner::with_clock(DEFAULT_WARN_INTERVAL, Arc::new(FixedClock));
    let (started_tx, started_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    logger
        .add_handler(Arc::new(BlockingHandler {
            started: started_tx,
            release: release_rx,
        }))
        .expect("native held handler should register");
    (logger, started_rx, release_tx)
}

/// Destruction reports drops that the warning interval is holding back.
///
/// The first refusal warns immediately and starts the interval, so the second
/// refusal is suppressed and stays pending. A flush must not report it - the
/// barrier is deadline-bound - but destruction has no caller deadline and must
/// say the count before the logger goes quiet for good.
#[test]
#[serial]
fn destruction_reports_drops_held_back_by_the_warning_interval() {
    let (logger, started_rx, release_tx) = blocked_logger();

    assert!(
        logger
            .log(FemtoLevel::Info, "occupies the only permit")
            .is_some(),
        "the first record should be accepted for dispatch",
    );
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("the worker should start handling the first record");

    assert!(logger.log(FemtoLevel::Info, "first refusal").is_some());
    assert!(logger.log(FemtoLevel::Info, "second refusal").is_some());
    let warnings = take_logged_messages();
    assert_eq!(
        count_warnings(
            &warnings,
            "FemtoLogger: dropped 1 records; dispatch capacity exhausted"
        ),
        1,
        "only the first refusal should warn inside the interval: {warnings:?}",
    );
    assert_eq!(
        logger.drop_warner.pending_drops(),
        1,
        "the suppressed refusal must stay pending rather than be discarded",
    );

    release_tx
        .send(())
        .expect("the worker should still be waiting for its first record");
    assert!(
        logger.flush_handlers(),
        "the accepted record should drain after the handler is released",
    );
    assert_eq!(
        logger.drop_warner.pending_drops(),
        1,
        "the deadline-bound flush must leave the count pending",
    );
    assert_eq!(
        count_warnings(
            &take_logged_messages(),
            "FemtoLogger: dropped 1 records; reported by flush",
        ),
        0,
        "the flush must not report the count synchronously",
    );

    drop(logger);

    assert_eq!(
        count_warnings(
            &take_logged_messages(),
            "FemtoLogger: dropped 1 records; reported by flush",
        ),
        1,
        "destruction should report the suppressed drop exactly once",
    );
}
