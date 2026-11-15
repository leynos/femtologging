//! Regression tests for file handler flush acknowledgement semantics.
//!
//! These tests ensure each flush call waits for its own worker
//! acknowledgement rather than reusing a stale ack from an earlier timed-out
//! flush.

use super::flush_ack_support::{BlockingFlushWriter, StalledFlushHandler, stalled_handler};
use super::*;
use rstest::*;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::handler::FlushContext;
use crate::monotonic_clock::SharedClock;
use crate::test_utils::flush_budget::SharedScriptedClock;

/// Ensure each flush waits on its own acknowledgement channel.
#[test]
fn flush_waits_for_its_own_acknowledgement() {
    let (started_tx, started_rx) = mpsc::channel();
    let writer = BlockingFlushWriter::new(started_tx);
    let mut config = TestConfig::new(writer.clone(), DefaultFormatter);
    config.capacity = 4;
    config.flush_interval = 16;
    config.overflow_policy = OverflowPolicy::Block;

    let handler = Arc::new(FemtoFileHandler::with_writer_for_test(config));

    let first_handler = Arc::clone(&handler);
    let (first_done_tx, first_done_rx) = mpsc::channel();
    let first_flush = thread::spawn(move || {
        first_done_tx
            .send(first_handler.flush())
            .expect("first flush result receiver must stay connected");
    });

    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first flush must reach the writer"),
        1
    );
    assert!(
        !first_done_rx
            .recv_timeout(Duration::from_millis(1_200))
            .expect("first flush must time out while the writer is blocked"),
        "first flush should fail once its private ack receiver times out",
    );
    first_flush
        .join()
        .expect("first flush worker thread must complete cleanly");

    writer.release_flushes_through(1);

    let second_handler = Arc::clone(&handler);
    let (second_done_tx, second_done_rx) = mpsc::channel();
    let second_flush = thread::spawn(move || {
        second_done_tx
            .send(second_handler.flush())
            .expect("second flush result receiver must stay connected");
    });

    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second flush must reach the writer"),
        2
    );
    assert!(
        second_done_rx
            // Slow CI workers can take longer to schedule the waiting flush,
            // so keep a wider margin before declaring the stale-ack guard
            // broken.
            .recv_timeout(Duration::from_millis(500))
            .is_err(),
        "second flush must wait for its own acknowledgement instead of consuming a stale ack",
    );

    writer.release_flushes_through(2);
    assert!(
        second_done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second flush must complete after its writer flush is released"),
        "second flush should succeed once its own ack arrives",
    );
    second_flush
        .join()
        .expect("second flush worker thread must complete cleanly");

    writer.release_all_flushes();
    let Ok(handler) = Arc::try_unwrap(handler) else {
        panic!("flush test must release all handler references before shutdown");
    };
    handler.close();
}

/// The supplied deadline, not the handler's one-second timeout, bounds an
/// acknowledgement the stalled worker never sends.
#[rstest]
fn flush_deadline_bounds_a_stalled_acknowledgement(mut stalled_handler: StalledFlushHandler) {
    assert_eq!(
        stalled_handler
            .started
            .recv_timeout(Duration::from_secs(1))
            .expect("the stalling flush must reach the writer"),
        1
    );

    // Admission has the whole budget; the acknowledgement has none.
    let start = Instant::now();
    let deadline = start + Duration::from_secs(30);
    let mut samples = 0_u8;
    let mut now = || {
        samples += 1;
        // The cap and admission checks see the start, so the enqueue gets the
        // whole budget. Only the acknowledgement sample sees it spent.
        if samples <= 2 {
            start
        } else {
            start + Duration::from_secs(31)
        }
    };
    let began = Instant::now();
    let confirmed = stalled_handler.handler().flush_by_at(deadline, &mut now);

    assert!(
        !confirmed,
        "a worker blocked inside the writer cannot acknowledge this flush"
    );
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "the acknowledgement wait must stop at the supplied deadline, not the one-second timeout"
    );
    // The acknowledgement wait ends at the supplied deadline, and the shared
    // helper then samples once more to check the answer it classified did not
    // arrive late. That fourth read is what turns a receive that took a ready
    // message with no budget left into a reported timeout.
    assert_eq!(
        samples, 4,
        "the cap, queue admission, acknowledgement and lateness check must each \
         sample the clock"
    );

    stalled_handler.writer.release_all_flushes();
    // The stalling flush has its own one-second budget, so whether the released
    // write finishes inside it is a scheduling detail. Only its completion
    // matters here; the assertion above covers the deadline under test.
    let outcome = stalled_handler.stall_outcome();
    assert!(
        outcome.is_some(),
        "the stalling flush must report its outcome once the writer is released"
    );
}

/// A caller deadline shorter than the handler's fixed one-second flush timeout
/// bounds the wait through the `flush_until` entry point.
#[rstest]
fn flush_until_lets_a_shorter_caller_deadline_bound_the_flush(
    mut stalled_handler: StalledFlushHandler,
) {
    assert_eq!(
        stalled_handler
            .started
            .recv_timeout(Duration::from_secs(1))
            .expect("the stalling flush must reach the writer"),
        1
    );

    // The caller allows fifty milliseconds; the handler's own budget is one
    // second. The second sample is already past the caller's deadline, so
    // neither queue wait may block.
    let clock = Arc::new(SharedScriptedClock::new(&[0, 100]));
    let injected: SharedClock = clock.clone();
    let context = FlushContext::with_clock(
        clock.started_at(),
        Some(clock.deadline_within(Duration::from_millis(50))),
        injected,
    );

    let began = Instant::now();
    let confirmed = stalled_handler.handler().flush_until(&context);
    let elapsed = began.elapsed();

    assert!(
        !confirmed,
        "a worker blocked inside the writer cannot acknowledge this flush"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "the caller's 50 ms deadline must bound the wait, not the one-second timeout"
    );
    // As above, the helper's lateness check adds the fourth context sample.
    assert_eq!(
        clock.samples(),
        4,
        "the cap, queue admission, acknowledgement and lateness check must each \
         sample the context clock"
    );

    stalled_handler.writer.release_all_flushes();
    let outcome = stalled_handler.stall_outcome();
    assert!(
        outcome.is_some(),
        "the stalling flush must report its outcome once the writer is released"
    );
}

/// A full queue must refuse the marker once the supplied deadline has no
/// budget left, even though the handler's timeout would still allow a wait.
#[rstest]
fn flush_deadline_bounds_a_stalled_queue_admission(
    #[with(1)] mut stalled_handler: StalledFlushHandler,
) {
    assert_eq!(
        stalled_handler
            .started
            .recv_timeout(Duration::from_secs(1))
            .expect("the stalling flush must reach the writer"),
        1
    );

    // The stalled worker cannot drain this record, so it holds the only slot.
    stalled_handler
        .handler()
        .handle(FemtoLogRecord::new(
            "flush-deadline",
            FemtoLevel::Info,
            "occupies the queue",
        ))
        .expect("the free slot should accept one record");

    let start = Instant::now();
    let deadline = start + Duration::from_secs(30);
    let mut samples = 0_u8;
    let mut now = || {
        samples += 1;
        start + Duration::from_secs(31)
    };
    let began = Instant::now();
    let confirmed = stalled_handler.handler().flush_by_at(deadline, &mut now);

    assert!(!confirmed, "a full queue cannot admit the flush marker");
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "queue admission must stop at the supplied deadline, not wait for room"
    );
    assert_eq!(
        samples, 2,
        "a refused admission must still sample the cap and the admission budget"
    );

    stalled_handler.writer.release_all_flushes();
    // As above: the stalling flush's own one-second budget decides its result,
    // which is not the deadline this test asserts on.
    let outcome = stalled_handler.stall_outcome();
    assert!(
        outcome.is_some(),
        "the stalling flush must report its outcome once the writer is released"
    );
}
