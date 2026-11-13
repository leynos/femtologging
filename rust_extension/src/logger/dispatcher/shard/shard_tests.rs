//! Drain-barrier classification tests for one shard's queue.
//!
//! A flush reports a closed queue and an unanswered barrier differently, and
//! the two waits that make up a barrier share one deadline. These tests drive
//! the shard directly: a logger-level test can reach the timeout path, but a
//! queue that no worker will ever drain is only reachable through the
//! test-only constructor here.

use std::any::Any;
use std::time::{Duration, Instant};

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::level::FemtoLevel;
use crate::log_record::FemtoLogRecord;
use crate::logger::QueuedRecord;

use super::*;

/// Time a stand-in worker leaves the queue full before making room.
///
/// Well inside one budget, so the acknowledgement wait still has most of it,
/// yet far enough outside [`SCHEDULING_TOLERANCE`] that a second budget would
/// push the call past the upper bound.
const ENQUEUE_SPEND: Duration = Duration::from_millis(900);

/// Allowance for thread scheduling on either side of one budget.
///
/// A two-budget implementation overruns it by a full `BARRIER_TIMEOUT` in the
/// tests that spend part of the budget before the acknowledgement wait.
const SCHEDULING_TOLERANCE: Duration = Duration::from_millis(600);

/// Build one queued record for *handler* now that no worker will run it.
fn queued_record(handler: &Arc<dyn FemtoHandlerTrait>) -> Box<QueuedRecord> {
    Box::new(QueuedRecord {
        record: FemtoLogRecord::new("shard.tests", FemtoLevel::Info, "queued"),
        handlers: vec![Arc::clone(handler)],
        #[cfg(feature = "python")]
        context: None,
    })
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

/// Handler that accepts a record without being called.
struct IdleHandler;

impl FemtoHandlerTrait for IdleHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn a_queue_without_a_receiver_reports_a_closed_channel() {
    let (shard, jobs) = Shard::without_worker_for_test(2);
    drop(jobs);

    assert_eq!(
        shard.wait_for_drain_outcome(),
        DrainOutcome::QueueClosed,
        "a queue no worker can read from must be reported as closed, not slow",
    );
}

#[test]
fn a_full_queue_reports_an_enqueue_timeout() {
    let (shard, _jobs) = Shard::without_worker_for_test(1);
    let budget = shard.budget();
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(IdleHandler);
    if shard.admit(queued_record(&handler), &budget).is_err() {
        panic!("a fresh one-deep shard should accept its first record");
    }

    let started = Instant::now();
    let outcome = shard.wait_for_drain_outcome();
    let elapsed = started.elapsed();

    assert_eq!(
        outcome,
        DrainOutcome::TimedOut,
        "a barrier that never finds room in the queue must time out",
    );
    assert!(
        elapsed >= BARRIER_TIMEOUT - SCHEDULING_TOLERANCE,
        "the enqueue wait must use its budget before giving up: {elapsed:?}",
    );
    // Queueing and acknowledging are two halves of one wait, so the call may
    // not outlast one budget by more than scheduling tolerance.
    assert!(
        elapsed < BARRIER_TIMEOUT + SCHEDULING_TOLERANCE,
        "the enqueue wait must not spend a second budget: {elapsed:?}",
    );
}

#[test]
fn a_blocked_worker_reports_an_acknowledgement_timeout() {
    let shard = Shard::start(0, 2).expect("test shard should start");
    let budget = shard.budget();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(HeldHandler {
        started: started_tx,
        release: parking_lot::Mutex::new(release_rx),
    });
    if shard.admit(queued_record(&handler), &budget).is_err() {
        panic!("a fresh test shard should accept its first record");
    }
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the worker should enter the blocking handler");

    let started = Instant::now();
    let outcome = shard.wait_for_drain_outcome();
    let elapsed = started.elapsed();

    assert_eq!(
        outcome,
        DrainOutcome::TimedOut,
        "a barrier behind a blocked handler must time out, not report a drain",
    );
    assert!(
        elapsed >= BARRIER_TIMEOUT - SCHEDULING_TOLERANCE,
        "the acknowledgement wait must use its budget before giving up: {elapsed:?}",
    );
    assert!(
        elapsed < BARRIER_TIMEOUT + SCHEDULING_TOLERANCE,
        "the acknowledgement wait must not spend a second budget: {elapsed:?}",
    );

    // Releasing the handler proves the shard is still usable after a timeout:
    // the second barrier drains instead of inheriting the first one's verdict.
    let _ = release_tx.send(());
    assert_eq!(shard.wait_for_drain_outcome(), DrainOutcome::Drained);
}

#[test]
fn an_enqueue_that_spends_the_budget_leaves_only_the_remainder_to_acknowledge() {
    let (shard, jobs) = Shard::without_worker_for_test(1);
    let budget = shard.budget();
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(IdleHandler);
    if shard.admit(queued_record(&handler), &budget).is_err() {
        panic!("a fresh one-deep shard should accept its first record");
    }

    let (release_tx, release_rx) = std::sync::mpsc::channel();
    // A stand-in worker holds the queue full for part of the budget, then takes
    // the barrier but withholds its acknowledgement. Holding the barrier job
    // keeps its acknowledgement sender alive, so the wait below sees a slow
    // worker rather than a closed channel. The worker's own waits are bounded
    // and never consult the code under test, so a deadline regression ends the
    // call here instead of hanging the suite.
    let worker = std::thread::spawn(move || {
        std::thread::sleep(ENQUEUE_SPEND);
        let record = jobs
            .recv_timeout(BARRIER_TIMEOUT * 2)
            .expect("the queued record should still be there");
        drop(record);
        let barrier = jobs
            .recv_timeout(BARRIER_TIMEOUT * 2)
            .expect("the barrier should arrive once the queue has room");
        let _ = release_rx.recv_timeout(BARRIER_TIMEOUT * 2);
        drop(barrier);
    });

    let started = Instant::now();
    let outcome = shard.wait_for_drain_outcome();
    let elapsed = started.elapsed();
    let _ = release_tx.send(());
    worker.join().expect("the stand-in worker should not panic");

    assert_eq!(
        outcome,
        DrainOutcome::TimedOut,
        "a worker that never answers must time out, not report a drain",
    );
    assert!(
        elapsed >= BARRIER_TIMEOUT - SCHEDULING_TOLERANCE,
        "the acknowledgement wait must use the deadline the enqueue left behind",
    );
    assert!(
        elapsed < BARRIER_TIMEOUT + SCHEDULING_TOLERANCE,
        "a fresh budget for the acknowledgement wait would exceed one budget: {elapsed:?}",
    );
}
