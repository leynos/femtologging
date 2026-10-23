//! Regression tests for the bounded `flush_handlers` barrier.
//!
//! The flush marker is a record queued behind everything the logger has
//! already accepted, so a saturated queue or a stalled handler is exactly the
//! state in which a flush has to give up rather than wait. These tests drive
//! that state directly, with a handler that holds the worker until the test
//! releases it, and assert the verdict together with the fact that the call
//! returned inside its own deadline.
//!
//! Before issue #438 the marker was enqueued with a blocking `send`, so the
//! full-queue test below did not fail — it never returned. The elapsed-time
//! assertion is what turns a hang into a reported failure, which is why these
//! tests read a clock and build outside `--cfg loom` only.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::handler::HandlerError;
use crate::sync::{Receiver, Sender, bounded};

use super::logger_tests_helpers::CollectingHandler;
use super::*;

/// The bound a flush is given, as a multiple of its own timeout.
///
/// The doubling is the scheduling allowance: a loaded host can delay the
/// wake-up that follows a timed wait, and several agents build here at once.
/// The defect this guards against is waiting *without* a bound, which no
/// finite allowance could mask, so a generous one still separates the two.
pub(super) fn flush_bound() -> Duration {
    Duration::from_millis(2 * LOGGER_FLUSH_TIMEOUT_MS)
}

/// A handler that holds the worker on its first record until released.
///
/// Later records pass straight through, so a test can drain a filled queue
/// once it has made its assertions. The release is a dropped sender rather
/// than a message, so if the test panics the unwind drops the sender and the
/// worker unblocks; a failed assertion therefore cannot wedge the logger's
/// `Drop` join and turn one failure into a hung test binary.
struct StallingHandler {
    /// Signals that the worker has entered `handle` and is now blocked.
    started: Sender<()>,
    /// Release channel: `recv` succeeds once the test sends, and errors once
    /// the test's sender is dropped.
    release: Receiver<()>,
    /// Set once `handle` has been unblocked, so later records do not block.
    released: AtomicBool,
}

impl FemtoHandlerTrait for StallingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        if !self.released.load(Ordering::SeqCst) {
            // A signal, not a rendezvous: the test has already taken the
            // acknowledgement it waits on, so a full or closed channel must
            // not turn a handler into a place the worker can block.
            let _ = self.started.try_send(());
            let _ = self.release.recv();
            self.released.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A handler whose `flush` reports failure.
struct FailingFlushHandler;

impl FemtoHandlerTrait for FailingFlushHandler {
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

/// Releases a stalled worker on drop, and after the bound if drop is late.
///
/// A test asserting that a flush gives up must run while the worker is still
/// stalled. If the implementation under test never returns, so does the test —
/// a hung binary rather than a failure, which is the very defect these tests
/// describe. The watchdog is the backstop: it releases the worker once the
/// bound has passed, so the flush returns and its elapsed-time assertion
/// reports what happened.
///
/// Both the drop and the watchdog may release, since either can lose the race
/// to the other. The worker treats the first release as the only one, and the
/// loser's send fails on a full channel, which is ignored.
struct StallGuard {
    /// Wakes the watchdog when the test has finished.
    done: Sender<()>,
    /// Makes the stalled worker continue.
    release: Sender<()>,
}

impl StallGuard {
    /// Take ownership of the release sender and arm the watchdog.
    fn new(release: Sender<()>) -> Self {
        let (done_tx, done_rx) = bounded(1);
        let watchdog = release.clone();
        std::thread::spawn(move || {
            // `Ok` is the test finishing; the timeout is it running past the
            // bound, which only an unbounded flush can cause.
            if done_rx.recv_timeout(flush_bound()).is_err() {
                let _ = watchdog.send(());
            }
        });
        Self {
            done: done_tx,
            release,
        }
    }
}

impl Drop for StallGuard {
    fn drop(&mut self) {
        let _ = self.done.try_send(());
        let _ = self.release.send(());
    }
}

/// Build a logger whose worker is blocked inside a stalling handler.
///
/// The guard releases the worker when the test ends, whether normally or by
/// unwinding, and after the deadline if the test is still waiting on a flush.
fn stalled_logger() -> (FemtoLogger, StallGuard) {
    let (started_tx, started_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(StallingHandler {
        started: started_tx,
        release: release_rx,
        released: AtomicBool::new(false),
    });
    let logger = FemtoLogger::new("flush".to_string());
    logger.add_handler(handler);
    logger.log(FemtoLevel::Info, "stall");
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker should enter the stalling handler");
    (logger, StallGuard::new(release_tx))
}

/// Fill the logger's queue and prove it is full.
///
/// The probe record is the proof: it is dropped only because the queue has no
/// room, which is the precondition this test exists to rely on. A silent
/// `log` call cannot establish it, because a logger whose worker is still
/// draining would accept every record and leave the flush with room to spare.
fn saturate_queue(logger: &FemtoLogger) {
    for _ in 0..DEFAULT_CHANNEL_CAPACITY {
        logger.log(FemtoLevel::Info, "fill");
    }
    logger.log(FemtoLevel::Info, "probe");
    assert_eq!(
        logger.get_dropped(),
        1,
        "the probe record should have been dropped, proving the queue is full"
    );
}

#[test]
fn flush_handlers_gives_up_on_a_full_queue_within_the_deadline() {
    let (logger, guard) = stalled_logger();
    saturate_queue(&logger);

    let started_at = Instant::now();
    let flushed = logger.flush_handlers();
    let elapsed = started_at.elapsed();

    assert!(
        !flushed,
        "a flush whose marker cannot be enqueued must not report success"
    );
    assert!(
        elapsed < flush_bound(),
        "flush waited {elapsed:?} on a full queue; the timeout is \
         {LOGGER_FLUSH_TIMEOUT_MS} ms and the bound is {:?}",
        flush_bound()
    );

    drop(guard);
}

#[test]
fn flush_handlers_gives_up_on_a_stalled_handler_within_the_deadline() {
    // The queue has room, so the marker is accepted; the worker is blocked on
    // the record ahead of it and cannot acknowledge.
    let (logger, guard) = stalled_logger();

    let started_at = Instant::now();
    let flushed = logger.flush_handlers();
    let elapsed = started_at.elapsed();

    assert!(
        !flushed,
        "a flush the worker never acknowledges must not report success"
    );
    assert!(
        elapsed < flush_bound(),
        "flush waited {elapsed:?} on a stalled handler; the timeout is \
         {LOGGER_FLUSH_TIMEOUT_MS} ms and the bound is {:?}",
        flush_bound()
    );

    drop(guard);
}

#[test]
fn flush_handlers_succeeds_once_the_worker_is_moving_again() {
    let (logger, guard) = stalled_logger();

    // Release the worker before flushing: the point here is the healthy path.
    let _ = guard.release.send(());

    let started_at = Instant::now();
    let flushed = logger.flush_handlers();
    let elapsed = started_at.elapsed();

    assert!(flushed, "a flush with a running worker should succeed");
    assert!(
        elapsed < flush_bound(),
        "a successful flush should not consume the whole budget, took {elapsed:?}"
    );
}

#[test]
fn flush_handlers_reports_a_handler_whose_flush_fails() {
    let handler = Arc::new(CollectingHandler::new()) as Arc<dyn FemtoHandlerTrait>;
    let failing = Arc::new(FailingFlushHandler) as Arc<dyn FemtoHandlerTrait>;
    let logger = FemtoLogger::new("flush".to_string());
    logger.add_handler(handler);
    logger.add_handler(failing);

    assert!(
        !logger.flush_handlers(),
        "a handler whose flush fails must make the logger's flush fail"
    );
}

#[test]
fn flush_handlers_succeeds_with_no_handlers() {
    let logger = FemtoLogger::new("flush".to_string());

    assert!(logger.flush_handlers());
}

#[test]
fn flush_handlers_fails_once_the_worker_has_exited() {
    let logger = FemtoLogger::new("flush".to_string());
    // Stop the worker without dropping the logger's sender, so the marker
    // meets a disconnected channel rather than a full one.
    logger
        .shutdown_tx
        .as_ref()
        .expect("a fresh logger has a shutdown sender")
        .send(())
        .expect("the worker should accept a shutdown signal");
    let handle = { logger.handle.lock().take() }.expect("a fresh logger has a worker");
    handle
        .join()
        .expect("the worker thread should exit cleanly");

    assert!(
        !logger.flush_handlers(),
        "a flush with no worker to acknowledge it must not report success"
    );
}
