//! Flush-deadline tests for the HTTP handler.
//!
//! The worker channel is built by the test and never read, so the worker
//! cannot answer a marker. Each test therefore asserts a property of the
//! caller's budget rather than of the network: an acknowledgement whose
//! budget was spent on admission returns at once, and a marker refused by a
//! full queue hands its deferred drop count back to the warner.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::bounded;

use super::{FemtoHTTPHandler, HTTPCommand};
use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::rate_limited_warner::RateLimitedWarner;
use crate::shutdown_signal::ShutdownSignal;
use crate::test_utils::flush_budget::{ScriptedClock, WarningClock};
use crate::test_utils::test_worker::TestWorker;

use crate::handlers::file::test_support::{install_test_logger, take_logged_messages};

/// Build a record for the tests.
fn record(message: &str) -> FemtoLogRecord {
    FemtoLogRecord::new("flush-budget", FemtoLevel::Info, message)
}

/// Fill one queue slot with a command the unread worker will never take.
fn occupy_one_slot(tx: &crossbeam_channel::Sender<HTTPCommand>) {
    let (ack, _ack_rx) = bounded(1);
    tx.try_send(HTTPCommand::Flush(ack, None))
        .expect("the queue should have a free slot");
}

/// Build a handler whose worker never reads its commands.
///
/// The receiving end is returned rather than dropped: it *is* the unread
/// worker, and dropping it would disconnect the queue the test needs full.
fn handler_with_unread_queue(
    capacity: usize,
    flush_timeout: Duration,
    warner: RateLimitedWarner,
) -> (
    FemtoHTTPHandler,
    crossbeam_channel::Sender<HTTPCommand>,
    crossbeam_channel::Receiver<HTTPCommand>,
) {
    let (tx, rx) = bounded(capacity);
    let signal = Arc::new(ShutdownSignal::new());
    let handle = thread::spawn(|| {});
    let handler = FemtoHTTPHandler::with_worker_for_test(
        tx.clone(),
        TestWorker::new(signal, handle),
        flush_timeout,
        warner,
    );
    (handler, tx, rx)
}

/// A budget already spent on admission leaves the acknowledgement nothing to
/// wait with, even though the handler's own timeout would still allow a wait.
#[test]
fn acknowledgement_stops_at_the_shared_deadline() {
    let (handler, tx, _unread) = handler_with_unread_queue(
        2,
        Duration::from_secs(1),
        RateLimitedWarner::new(Duration::from_secs(1)),
    );
    // One queue slot is now taken, leaving the other for the flush marker.
    occupy_one_slot(&tx);

    // The cap and admission checks see the start, so the marker is admitted
    // with the whole handler budget. Only the acknowledgement sample sees that
    // budget spent.
    let mut clock = ScriptedClock::new(&[0, 0, 30_001]);
    let deadline = clock.deadline();
    let began = Instant::now();
    let confirmed = handler.flush_by_at(deadline, &mut || clock.now());

    assert!(!confirmed, "an unread worker cannot acknowledge this flush");
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "the acknowledgement must stop at the spent budget, not wait out the handler timeout"
    );
    assert_eq!(
        clock.samples(),
        3,
        "the cap, queue admission and acknowledgement must each sample the clock"
    );

    let mut handler = handler;
    handler.close();
}

/// A flush whose marker never enters a saturated queue must keep its due drop
/// count for the next warning, rather than swallowing it with the failed flush.
#[test]
fn failed_flush_enqueue_reports_its_drops_on_the_next_warning() {
    install_test_logger();
    let clock = Arc::new(WarningClock::default());
    let warner = RateLimitedWarner::with_clock(Duration::from_secs(5), clock.clone());
    let (mut handler, tx, _unread) =
        handler_with_unread_queue(1, Duration::from_millis(100), warner);
    occupy_one_slot(&tx);

    // The first drop is due and warns at once; the next two stay pending
    // because the interval has not elapsed.
    for _ in 0..3 {
        assert_eq!(
            FemtoHandlerTrait::handle(&handler, record("dropped")),
            Err(HandlerError::QueueFull),
            "a full queue must refuse every record"
        );
    }

    // Let the interval elapse so the next flush is the vehicle for the count.
    clock.advance(Duration::from_secs(5));
    take_logged_messages();
    assert!(
        !handler.flush(),
        "a full queue must refuse the flush marker"
    );

    // One more dropped record makes the next warning due. The count it carries
    // is the restored one; the record dropped here supplies only one.
    assert_eq!(
        FemtoHandlerTrait::handle(&handler, record("dropped after the flush")),
        Err(HandlerError::QueueFull)
    );
    let warnings = take_logged_messages();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("dropped 2 records")),
        "the restored count must reach the next warning, saw {warnings:?}"
    );

    handler.close();
}
