//! Contracts for rejecting records on the logger's producer path.
//!
//! A logger decides what each shard receives before any handler runs. These
//! tests pin the two ways that decision loses a record: a handler filter that
//! declines it must not enqueue an empty job for the worker, and a shard that
//! refuses admission must count the record as dropped and report it through the
//! logger's warner.
//!
//! The rest of the file covers the reporting half of that contract. Drops the
//! warning interval holds back must still be reported by a flush - both for a
//! count that is already pending when the flush starts, and for drops recorded
//! while the queue drains - because a logger being shut down may have no due
//! moment left.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::thread;
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

/// A flush reports drops that the warning interval is holding back.
///
/// The first refusal warns immediately and starts the interval, so the second
/// refusal is suppressed and stays pending. A flush must say that pending
/// count now: on a logger being shut down there may be no due moment left.
#[test]
#[serial]
fn flush_reports_drops_held_back_by_the_warning_interval() {
    install_test_logger();
    let shard: &'static Shard = Box::leak(Box::new(
        Shard::start(0, 1).expect("a one-record test shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("producer-flush".to_string(), shard);
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

    assert!(logger.log(FemtoLevel::Info, "first refusal").is_some());
    assert!(logger.log(FemtoLevel::Info, "second refusal").is_some());
    let warnings = take_logged_messages();
    let suppression_warnings = warnings
        .iter()
        .filter(|warning| {
            warning.level == log::Level::Warn
                && warning.message == "FemtoLogger: dropped 1 records; dispatch capacity exhausted"
        })
        .count();
    assert_eq!(
        suppression_warnings, 1,
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
        0,
        "the flush must drain the pending count",
    );
    let flush_warnings = take_logged_messages()
        .into_iter()
        .filter(|warning| {
            warning.level == log::Level::Warn
                && warning.message == "FemtoLogger: dropped 1 records; reported by flush"
        })
        .count();
    assert_eq!(
        flush_warnings, 1,
        "the flush should report the suppressed drop exactly once",
    );
}

/// The channel ends a mid-drain flush test must release.
///
/// Held as one value so the scope owns them: an assertion failure drops them
/// while unwinding, which disconnects the hook and the blocked handler instead
/// of leaving `thread::scope` waiting on threads only the failed test could
/// feed.
struct FlushRelease {
    /// Releases the handler holding the only dispatch permit.
    release_handler: Sender<()>,
    /// Signalled by the test-only hook once the flush is parked.
    at_drain: Receiver<()>,
    /// Released when the test is ready for the flush to continue.
    resume_flush: Sender<()>,
}

/// Build a logger whose next flush parks at the drain wait.
///
/// The queue is full, one record is in flight behind a blocked handler, and
/// one drop is pending from a suppressed refusal, so the flush that follows is
/// provably mid-drain when the hook fires.
fn prepare_parked_flush() -> (FemtoLogger, FlushRelease) {
    let shard: &'static Shard = Box::leak(Box::new(
        Shard::start(0, 1).expect("a one-record test shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("producer-drain-flush".to_string(), shard);
    let (started_tx, started_rx) = bounded(1);
    let (release_handler, release_rx) = bounded(1);
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

    // The first refusal warns and starts the interval; the second is
    // suppressed, so one drop is pending when the flush begins.
    assert!(logger.log(FemtoLevel::Info, "first refusal").is_some());
    assert!(logger.log(FemtoLevel::Info, "second refusal").is_some());
    let _ = take_logged_messages();
    assert_eq!(logger.drop_warner.pending_drops(), 1);

    // The hook parks the flushing thread exactly where the drain wait begins,
    // so the refusal below is provably mid-drain. Sleeping here instead would
    // be slow when it works and flaky when the flush thread is descheduled.
    let (at_drain_tx, at_drain) = bounded(1);
    let (resume_flush, resume_flush_rx) = bounded(1);
    logger.set_drain_hook_for_test(move || {
        let _ = at_drain_tx.send(());
        // Bounded, so a hook left waiting by an already-failed test returns
        // instead of parking the flush thread where `thread::scope` joins it.
        let _ = resume_flush_rx.recv_timeout(Duration::from_secs(5));
    });

    (
        logger,
        FlushRelease {
            release_handler,
            at_drain,
            resume_flush,
        },
    )
}

/// Assert one flush reported both the suppressed and the mid-drain drop.
fn assert_flush_reported_the_drain(logger: &FemtoLogger) {
    assert_eq!(
        logger.drop_warner.pending_drops(),
        0,
        "the flush must report the drop recorded while the queue drained",
    );
    let flush_warnings: Vec<_> = take_logged_messages()
        .into_iter()
        .filter(|warning| {
            warning.level == log::Level::Warn
                && warning.message == "FemtoLogger: dropped 2 records; reported by flush"
        })
        .collect();
    assert_eq!(
        flush_warnings.len(),
        1,
        "one flush warning should carry both pending drops",
    );
}

/// A drop recorded while the dispatch queue is draining is still reported by
/// the same explicit flush.
///
/// `flush_handlers_blocking` reports drop warnings only after the drain. A
/// producer refused during the wait records its drop after any earlier call to
/// the warners, and `RateLimitedWarner::flush` reports the counts present when
/// it runs, so flushing first would return with that drop still pending.
#[test]
#[serial]
fn flush_reports_drops_recorded_while_the_queue_drains() {
    install_test_logger();
    let (logger, release) = prepare_parked_flush();
    let logger_ref = &logger;
    thread::scope(move |scope| {
        let flush = scope.spawn(move || logger_ref.flush_handlers());
        release
            .at_drain
            .recv_timeout(Duration::from_secs(5))
            .expect("the flush should reach its drain wait");
        assert!(
            logger_ref
                .log(FemtoLevel::Info, "refusal during the drain")
                .is_some(),
            "the full queue should refuse the record during the drain",
        );
        release
            .release_handler
            .send(())
            .expect("the worker should still be waiting for its first record");
        release
            .resume_flush
            .send(())
            .expect("the flush should still be waiting to be released");
        assert!(
            flush.join().expect("the flush thread should not panic"),
            "the accepted record should drain after the handler is released",
        );
        assert_flush_reported_the_drain(logger_ref);
    });
}
