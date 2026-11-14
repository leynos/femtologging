//! Tests for the flush measurement seam and the orchestration it measures.
//!
//! Two properties are checked here. The command itself reads the clock once
//! either side of an attempt and records exactly one outcome, which a manual
//! clock can pin to an exact duration. And the orchestration the production
//! wrapper calls — drain the shard, then flush the handlers — produces each of
//! the four terminal outcomes from a real logger, so the classification is
//! exercised end to end rather than by a closure that returns a preset value.
//!
//! Neither test touches the process-wide counters: every measurement lands in
//! an observer the test owns, so a parallel test binary cannot move an
//! assertion.

use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rstest::rstest;

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::logger::{FemtoLogger, Shard, flush_metrics::FlushMetrics};
use crate::monotonic_clock::{ManualClock, MonotonicClock};

use super::{FlushObserver, FlushOutcome, MetricsFlushObserver, measure_flush_attempt};

/// How far the fake clock moves while one attempt runs.
const ATTEMPT_DURATION: Duration = Duration::from_millis(250);

/// How long a test-side wait may take before it is reported as a hang.
const WAIT_BOUND: Duration = Duration::from_secs(3);

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

/// Clock whose readings step by a fixed amount regardless of real time.
///
/// A real attempt can block for the whole barrier timeout, so a manual clock
/// that only advances on request would report zero elapsed time for the
/// timeout cases. This clock advances on every reading instead, which lets the
/// orchestration tests assert the exact value the injected clock measured
/// while the attempt takes as long as it needs.
struct SteppingClock {
    /// Value of the first reading.
    base: Instant,
    /// Amount added per reading.
    step: Duration,
    /// How many readings have been handed out.
    reads: AtomicUsize,
}

impl SteppingClock {
    /// Start a clock whose readings advance by `step`.
    fn new(step: Duration) -> Self {
        Self {
            base: Instant::now(),
            step,
            reads: AtomicUsize::new(0),
        }
    }
}

impl MonotonicClock for SteppingClock {
    fn now(&self) -> Instant {
        let read = self.reads.fetch_add(1, Ordering::SeqCst);
        let steps = u32::try_from(read).unwrap_or(u32::MAX);
        self.base + self.step * steps
    }
}

/// Handler whose flush always succeeds.
struct PassingHandler;

impl FemtoHandlerTrait for PassingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Handler whose flush reports failure.
struct FailingHandler;

impl FemtoHandlerTrait for FailingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Handler that reports entering `handle` and then blocks until released.
struct HeldHandler {
    /// Notifies the test that the worker entered `handle`.
    started: std::sync::mpsc::Sender<()>,
    /// Keeps `handle` blocked until the test releases the worker.
    release: parking_lot::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl FemtoHandlerTrait for HeldHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        let _ = self.started.send(());
        let _ = self.release.lock().recv();
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Leak a logger's shard, because a logger borrows it for `'static`.
fn leaked(shard: Shard) -> &'static Shard {
    Box::leak(Box::new(shard))
}

/// Build a logger on a freshly started two-deep shard.
fn logger_on_test_shard(name: &str) -> FemtoLogger {
    let shard = leaked(Shard::start(0, 2).expect("test shard should start"));
    FemtoLogger::assembled_for_test(name.to_string(), shard)
}

/// Attach one handler and report the logger.
fn with_handler(logger: FemtoLogger, handler: Arc<dyn FemtoHandlerTrait>) -> FemtoLogger {
    logger
        .add_handler(handler)
        .expect("the test handler should register");
    logger
}

/// Every terminal outcome is measured and reported unchanged.
///
/// The closure here *is* the attempt, so this pins the command's own contract:
/// one clock reading either side, one record, and the attempt's outcome
/// returned unchanged. The tests below cover the outcomes the real
/// orchestration produces.
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

/// A drained shard with a passing handler is classified as a success.
#[test]
fn a_drained_shard_flushing_its_handlers_reports_success() {
    let logger = with_handler(
        logger_on_test_shard("flush.success"),
        Arc::new(PassingHandler),
    );
    let clock = SteppingClock::new(ATTEMPT_DURATION);
    let observer = RecordingObserver::default();

    let outcome = logger.flush_handlers_measured(&clock, &observer);

    assert_eq!(outcome, FlushOutcome::Success);
    assert_eq!(
        observer.taken(),
        [(FlushOutcome::Success, ATTEMPT_DURATION)],
        "the real attempt must be measured once with the injected clock",
    );
}

/// A drained shard with a failing handler is classified as a handler failure.
#[test]
fn a_failing_handler_flush_reports_a_handler_failure() {
    let logger = with_handler(
        logger_on_test_shard("flush.handler"),
        Arc::new(FailingHandler),
    );
    let clock = SteppingClock::new(ATTEMPT_DURATION);
    let observer = RecordingObserver::default();

    let outcome = logger.flush_handlers_measured(&clock, &observer);

    assert_eq!(outcome, FlushOutcome::HandlerFailure);
    assert_eq!(
        observer.taken(),
        [(FlushOutcome::HandlerFailure, ATTEMPT_DURATION)],
        "the classification must describe the attempt that actually ran",
    );
}

/// A shard whose worker is blocked classifies the drain wait as a timeout.
#[test]
fn a_blocked_worker_reports_an_acknowledgement_timeout() {
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let handler = Arc::new(HeldHandler {
        started: started_tx,
        release: parking_lot::Mutex::new(release_rx),
    });
    let logger = with_handler(logger_on_test_shard("flush.timeout"), handler);
    logger.log(FemtoLevel::Info, "held by the blocking handler");
    started_rx
        .recv_timeout(WAIT_BOUND)
        .expect("the worker should enter the blocking handler");

    let clock = SteppingClock::new(ATTEMPT_DURATION);
    let observer = RecordingObserver::default();
    let outcome = logger.flush_handlers_measured(&clock, &observer);

    // Release before asserting so a failure cannot leave a blocked worker.
    release_tx
        .send(())
        .expect("the worker should still be waiting to be released");

    assert_eq!(outcome, FlushOutcome::AckTimeout);
    assert_eq!(
        observer.taken(),
        [(FlushOutcome::AckTimeout, ATTEMPT_DURATION)],
        "the timeout must be measured once, not once per wait it performs",
    );
}

/// A shard whose worker refuses the barrier reports a closed channel.
#[test]
fn a_refused_barrier_reports_a_closed_channel() {
    let (shard, worker) = Shard::with_unanswering_worker_for_test(2);
    let logger = with_handler(
        FemtoLogger::assembled_for_test("flush.closed".to_string(), leaked(shard)),
        Arc::new(PassingHandler),
    );
    // The record stays in the stand-in worker's hands, holding its permit, so
    // the logger has something in flight and must ask the shard to drain.
    logger.log(FemtoLevel::Info, "held in flight");
    worker.wait_until_record_held(WAIT_BOUND);

    let clock = SteppingClock::new(ATTEMPT_DURATION);
    let observer = RecordingObserver::default();
    let outcome = logger.flush_handlers_measured(&clock, &observer);

    // The guard releases the worker and joins it before any assertion runs.
    drop(worker);

    assert_eq!(outcome, FlushOutcome::ChannelClosed);
    assert_eq!(
        observer.taken(),
        [(FlushOutcome::ChannelClosed, ATTEMPT_DURATION)],
        "a refused barrier must be measured once, as the drain classification",
    );
}

/// The metrics observer counts exactly what it is handed.
///
/// The target is a metrics instance this test owns, so no other logger in the
/// test binary can move the counts.
#[test]
fn the_metrics_observer_counts_the_outcome_it_is_handed() {
    let metrics = FlushMetrics::new();
    let observer = MetricsFlushObserver::new(&metrics);

    observer.record(FlushOutcome::ChannelClosed, Duration::from_micros(7));

    let snapshot = metrics.snapshot();
    assert_eq!(
        snapshot.outcomes,
        [0, 0, 1, 0],
        "only the handed outcome may be counted",
    );
    assert_eq!(snapshot.duration_count, 1, "one duration was recorded");
    assert_eq!(
        snapshot.duration_sum_us, 7,
        "the exact duration was recorded"
    );
}
