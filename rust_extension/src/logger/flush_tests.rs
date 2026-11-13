//! Command tests for the flush measurement seam.
//!
//! The production wrapper is exercised through a real logger by the logger
//! tests. These tests drive the orchestration directly, so the clock and the
//! observer are the test's own and an assertion can attribute an outcome and
//! an elapsed time to one flush rather than to a process-wide counter.

use std::sync::Mutex;
use std::time::Duration;

use rstest::rstest;

use crate::monotonic_clock::ManualClock;

use super::{
    FlushObserver, FlushOutcome, flush_metrics_snapshot, measure_flush_attempt,
    production_flush_observer,
};

/// How far the fake clock moves while one attempt runs.
const ATTEMPT_DURATION: Duration = Duration::from_millis(250);

/// Observer that keeps every measurement it is handed.
#[derive(Default)]
struct RecordingObserver {
    /// Measurements in the order the attempt reported them.
    records: Mutex<Vec<(FlushOutcome, Duration)>>,
}

impl FlushObserver for RecordingObserver {
    fn record(&self, outcome: FlushOutcome, elapsed: Duration) {
        self.records
            .lock()
            .expect("the recording observer is not shared with a panicking thread")
            .push((outcome, elapsed));
    }
}

impl RecordingObserver {
    /// Take everything recorded so far.
    fn taken(&self) -> Vec<(FlushOutcome, Duration)> {
        self.records
            .lock()
            .expect("the recording observer is not shared with a panicking thread")
            .drain(..)
            .collect()
    }
}

/// Every terminal outcome is measured and reported unchanged.
#[rstest]
#[case::success(FlushOutcome::Success)]
#[case::ack_timeout(FlushOutcome::AckTimeout)]
#[case::channel_closed(FlushOutcome::ChannelClosed)]
#[case::handler_failure(FlushOutcome::HandlerFailure)]
fn one_attempt_records_its_outcome_and_elapsed_time(#[case] expected: FlushOutcome) {
    let clock = ManualClock::new();
    let observer = RecordingObserver::default();

    let measured = measure_flush_attempt(&clock, &observer, || {
        clock.advance(ATTEMPT_DURATION);
        expected
    });

    assert_eq!(
        measured, expected,
        "the command must return the attempt's own outcome",
    );
    assert_eq!(
        observer.taken(),
        [(expected, ATTEMPT_DURATION)],
        "one attempt must record exactly one outcome and the time the clock measured",
    );
}

/// The production observer forwards one record to the process-wide counters.
///
/// This is a delta around a single direct call, so the assertion is
/// attributable to the record handed to the observer: nothing else in the
/// test binary records a flush at this point.
#[test]
fn the_production_observer_forwards_to_the_process_metrics() {
    let before = flush_metrics_snapshot();
    production_flush_observer().record(FlushOutcome::ChannelClosed, Duration::from_micros(7));
    let after = flush_metrics_snapshot();

    assert_eq!(
        after.outcomes[FlushOutcome::ChannelClosed.index()],
        before.outcomes[FlushOutcome::ChannelClosed.index()] + 1,
        "the production observer must count the outcome it was handed",
    );
    assert_eq!(
        after.duration_count,
        before.duration_count + 1,
        "the production observer must record one duration",
    );
    assert_eq!(
        after.duration_sum_us,
        before.duration_sum_us + 7,
        "the recorded duration must be the one the observer was handed",
    );
}
