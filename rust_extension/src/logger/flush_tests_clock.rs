//! Clock-driven flush-barrier tests.
//!
//! These tests supply the barrier's clock directly, so each asserts how the
//! shared deadline is spent rather than how fast the machine happens to be.
//! The helpers live in the parent module, which also uses them for the
//! generated-time property test.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{
    FixedClock, ResultHandler, SequenceClock, TEST_FLUSH_BUDGET, assert_single_flush_failure,
};
use crate::logger::FemtoLogger;
use crate::logger::flush_diagnostics::FlushFailureStage;

/// Drive one flush from scripted clock offsets and assert where it stopped.
///
/// Each offset is the elapsed time the barrier sees at that sample; a sample
/// of [`TEST_FLUSH_BUDGET`] therefore lands exactly on the deadline.
fn assert_flush_fails_at_clock_offsets(
    logger: &FemtoLogger,
    clock_offsets: &[Duration],
    expected_stage: FlushFailureStage,
) {
    let started_at = Instant::now();
    let deadline = started_at + TEST_FLUSH_BUDGET;
    let samples = clock_offsets
        .iter()
        .map(|offset| started_at + *offset)
        .collect();
    let clock = Arc::new(SequenceClock {
        samples: std::sync::Mutex::new(samples),
        fallback: deadline,
    });
    let outcome = logger.flush_handlers_with_clock(TEST_FLUSH_BUDGET, clock);

    assert!(!outcome.finish(
        &logger.flush_diagnostics,
        FlushFailureStage::CallerReturnDeadlineExceeded,
    ));
    assert_single_flush_failure(&logger.flush_diagnostics, expected_stage);
}

#[test]
fn injected_clock_reaches_the_worker_with_one_deadline() {
    let logger = FemtoLogger::new("flush-injected-clock".to_owned())
        .expect("diagnostic test logger should be constructible");
    logger
        .add_handler(Arc::new(ResultHandler { confirmed: true }))
        .expect("test handler should be accepted");
    let started_at = Instant::now();
    let outcome = logger.flush_handlers_with_clock(
        Duration::from_secs(1),
        Arc::new(FixedClock { now: started_at }),
    );

    assert!(outcome.finish(
        &logger.flush_diagnostics,
        FlushFailureStage::CallerReturnDeadlineExceeded,
    ));
}

#[test]
fn injected_clock_bounds_handler_snapshot_acquisition() {
    let logger = FemtoLogger::new("flush-snapshot-clock".to_owned())
        .expect("diagnostic test logger should be constructible");
    let _handler_guard = logger.handlers.write();
    assert_flush_fails_at_clock_offsets(
        &logger,
        &[Duration::ZERO, Duration::ZERO, TEST_FLUSH_BUDGET],
        FlushFailureStage::HandlerSnapshotTimedOut,
    );
}

#[test]
fn a_contended_handler_snapshot_releases_the_shard_worker() {
    let logger = FemtoLogger::new("flush-contended-snapshot".to_owned())
        .expect("contended snapshot logger should be constructible");
    logger
        .add_handler(Arc::new(ResultHandler { confirmed: true }))
        .expect("test handler should be accepted");

    let began = Instant::now();
    {
        let _handler_guard = logger.handlers.write();
        let outcome = logger.flush_handlers_with_clock(
            Duration::from_millis(50),
            crate::monotonic_clock::system_clock(),
        );
        assert!(
            !outcome.finish(
                &logger.flush_diagnostics,
                FlushFailureStage::CallerReturnDeadlineExceeded,
            ),
            "a registry held for the whole budget must fail the flush"
        );
    }
    assert!(
        began.elapsed() < Duration::from_secs(1),
        "the snapshot wait must stop at the budget, not at the registry holder"
    );
    assert_single_flush_failure(
        &logger.flush_diagnostics,
        FlushFailureStage::HandlerSnapshotTimedOut,
    );

    // Dropping the guard disconnects the pending marker's start gate, so the
    // shard worker is free to answer the next flush rather than waiting out a
    // budget that has already been spent.
    assert!(
        logger.flush_handlers(),
        "the shard worker must recover once the handler registry is released"
    );
}

#[test]
fn injected_clock_stops_before_marker_when_the_shared_deadline_expires() {
    let logger = FemtoLogger::new("flush-marker-clock".to_owned())
        .expect("diagnostic test logger should be constructible");
    assert_flush_fails_at_clock_offsets(
        &logger,
        &[
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            TEST_FLUSH_BUDGET,
        ],
        FlushFailureStage::DeadlineExpiredBeforeMarker,
    );
}
