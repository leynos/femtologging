//! Flush-deadline tests for the HTTP handler.
//!
//! The worker channel is built by the test. Most tests leave it unread, so the
//! worker cannot answer a marker, and each asserts a property of the caller's
//! budget rather than of the network: an acknowledgement whose budget was spent
//! on admission returns at once, and a marker refused by a full queue hands its
//! deferred drop count back to the warner. One test supplies a worker that does
//! answer, and shows that an answer the caller only observes after the deadline
//! is refused rather than counted as confirmed.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::bounded;

use super::{FemtoHTTPHandler, HTTPCommand};
use crate::handler::{FemtoHandlerTrait, FlushContext, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::monotonic_clock::SharedClock;
use crate::rate_limited_warner::RateLimitedWarner;
use crate::shutdown_signal::ShutdownSignal;
use crate::test_utils::flush_budget::{ScriptedClock, SharedScriptedClock, WarningClock};
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

/// Build a handler whose worker answers every flush marker at once.
///
/// The answer is therefore genuinely ready when the caller's receive begins,
/// which is what separates the lateness rule from an ordinary timeout: the
/// caller really receives an acknowledgement and still has to refuse it.
fn handler_with_answering_worker(capacity: usize, flush_timeout: Duration) -> FemtoHTTPHandler {
    let (tx, rx) = bounded(capacity);
    let worker = thread::spawn(move || {
        // Any other command, such as the shutdown request, ends the loop; the
        // handler treats the closed channel as a worker that has already left.
        while let Ok(HTTPCommand::Flush(ack, _pending_drops)) = rx.recv() {
            let _ = ack.send(());
        }
    });
    let signal = Arc::new(ShutdownSignal::new());
    FemtoHTTPHandler::with_worker_for_test(
        tx,
        TestWorker::new(signal, worker),
        flush_timeout,
        RateLimitedWarner::new(Duration::from_secs(1)),
    )
}

/// Build a handler whose worker answers every flush marker and records the
/// pending drop count that marker carried.
///
/// The counts travel out of the worker rather than being logged, so the test
/// observes exactly what the flush handed to the queue.
fn handler_with_recording_worker(
    capacity: usize,
    flush_timeout: Duration,
    warner: RateLimitedWarner,
) -> (FemtoHTTPHandler, crossbeam_channel::Receiver<Option<u64>>) {
    let (tx, rx) = bounded(capacity);
    let (seen_tx, seen_rx) = bounded(capacity);
    let worker = thread::spawn(move || {
        while let Ok(HTTPCommand::Flush(ack, pending_drops)) = rx.recv() {
            let _ = seen_tx.send(pending_drops);
            let _ = ack.send(());
        }
    });
    let signal = Arc::new(ShutdownSignal::new());
    let handler = FemtoHTTPHandler::with_worker_for_test(
        tx,
        TestWorker::new(signal, worker),
        flush_timeout,
        warner,
    );
    (handler, seen_rx)
}

/// An answer the caller only observes after the deadline must not confirm the
/// flush, even though the worker sent it and the receive took it.
///
/// The zero-budget variant — an answer ready *before* the wait begins, received
/// with nothing left to wait with — is pinned in `handler_tests.rs`, because a
/// live worker cannot be scheduled to have answered that early. Here the answer
/// is genuinely received under a whole budget and only the lateness sample sees
/// the deadline passed, which is the case this handler can schedule.
#[test]
fn a_ready_acknowledgement_observed_after_the_deadline_does_not_confirm() {
    let handler = handler_with_answering_worker(2, Duration::from_secs(1));

    // The first three samples sit at the start: the marker is admitted, the
    // answer is really received, and the handler's one-second budget is still
    // whole for both waits. Only the lateness sample sees the deadline passed.
    let mut clock = ScriptedClock::new(&[0, 0, 0, 31_000]);
    let deadline = clock.deadline();
    let began = Instant::now();
    let confirmed = handler.flush_by_at(deadline, &mut || clock.now());

    assert!(
        !confirmed,
        "an answer observed after the deadline must not confirm the flush"
    );
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "the refusal must not wait out the handler's timeout"
    );
    assert_eq!(
        clock.samples(),
        4,
        "the cap, queue admission, acknowledgement and lateness check must each sample the clock"
    );

    let mut handler = handler;
    handler.close();
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

    // Admission spends 600 ms of the handler's one-second budget, so the
    // marker is admitted with only the 400 ms that remain, and the
    // acknowledgement is offered what is left of the same deadline. That runs
    // out at the third sample, which is why the wait returns at once instead
    // of waiting out the full handler timeout.
    let mut clock = ScriptedClock::new(&[0, 600, 1_000]);
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

/// A caller deadline shorter than the handler's own timeout bounds the flush
/// through the `flush_until` entry point the logger barrier calls.
#[test]
fn a_caller_deadline_shorter_than_the_handler_timeout_bounds_the_flush() {
    let (handler, _tx, unread) = handler_with_unread_queue(
        2,
        Duration::from_secs(2),
        RateLimitedWarner::new(Duration::from_secs(1)),
    );
    // The caller allows fifty milliseconds; the handler would allow two
    // seconds. The second sample is already past the caller's deadline, so
    // neither queue wait may block.
    let clock = Arc::new(SharedScriptedClock::new(&[0, 100]));
    let injected: SharedClock = clock.clone();
    let context = FlushContext::with_clock(
        clock.started_at(),
        Some(clock.deadline_within(Duration::from_millis(50))),
        injected,
    );

    let began = Instant::now();
    let confirmed = handler.flush_until(&context);
    let elapsed = began.elapsed();

    assert!(!confirmed, "an unread worker cannot acknowledge this flush");
    assert!(
        elapsed < Duration::from_secs(1),
        "the caller's 50 ms deadline must bound the wait, not the handler's two-second timeout"
    );
    assert_eq!(
        clock.samples(),
        3,
        "the cap, queue admission and acknowledgement must each sample the context clock"
    );

    // Dropping the unread receiver disconnects the queue, so closing the
    // handler takes the immediate path instead of waiting out its timeout.
    drop(unread);
    let mut handler = handler;
    handler.close();
}

/// An explicit flush must carry a drop recorded inside the warning interval.
///
/// The interval limits how often ordinary warnings are emitted; it does not
/// make a drop unreportable. The first drop warns through the ordinary path,
/// the second is recorded half an interval later, and the warning that follows
/// would be suppressed, so only the flush can report the second count.
#[test]
fn flush_carries_a_drop_recorded_inside_the_warning_interval() {
    let clock = Arc::new(WarningClock::default());
    let warner = RateLimitedWarner::with_clock(Duration::from_secs(5), clock.clone());
    let (mut handler, counts) =
        handler_with_recording_worker(2, Duration::from_millis(200), warner);

    handler.warner.record_drop();
    handler.warner.warn_if_due(|_| {});
    clock.advance(Duration::from_millis(500));
    handler.warner.record_drop();

    assert!(
        handler.flush(),
        "the recording worker acknowledges the flush marker"
    );
    let carried = counts.recv_timeout(Duration::from_secs(1));
    assert_eq!(
        carried,
        Ok(Some(1)),
        "the marker must carry the drop recorded inside the interval"
    );

    handler.close();
}

/// A flush whose marker never enters a saturated queue must keep its due drop
/// count for the next warning, rather than swallowing it with the failed flush.
///
/// The installed test logger is process-wide, so this test must not overlap a
/// sibling that reads or replaces the same buffer.
#[test]
#[serial_test::serial]
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
