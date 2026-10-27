//! Regression tests for the bounded `flush_handlers` barrier.
//!
//! A shard barrier shares the same bounded queue as dispatch jobs. The tests
//! hold the worker inside a handler, then prove that a queued marker fills a
//! deliberately shallow queue before another flush attempts insertion.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::log_record::FemtoLogRecord;
use crate::logger::{FemtoLogger, Shard};
use crate::sync::{Receiver, Sender, bounded};

/// The bound a flush is given, as a multiple of its own timeout.
pub(super) fn flush_bound() -> Duration {
    Duration::from_millis(2 * super::super::LOGGER_FLUSH_TIMEOUT_MS)
}

/// A handler that holds the shard worker until the test releases it.
struct StallingHandler {
    /// Signals that the worker has entered `handle` and is blocked.
    started: Sender<()>,
    /// Releases the worker when a message arrives or the sender is dropped.
    release: Receiver<()>,
    /// Lets later records pass through after the first release.
    released: AtomicBool,
}

impl FemtoHandlerTrait for StallingHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        if !self.released.load(Ordering::SeqCst) {
            let _ = self.started.try_send(());
            let _ = self.release.recv();
            self.released.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn flush_until(&self, _deadline: Instant) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A handler whose flush explicitly reports failure.
struct FailingFlushHandler;

impl FemtoHandlerTrait for FailingFlushHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, _deadline: Instant) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Releases a stalled worker during normal cleanup or after a broken wait.
pub(super) struct StallGuard {
    /// Wakes the watchdog when the test ends.
    done: Sender<()>,
    /// Makes the stalled worker continue.
    release: Sender<()>,
    /// Joins the watchdog so it cannot outlive the test.
    watchdog: Option<std::thread::JoinHandle<()>>,
}

impl StallGuard {
    /// Take ownership of the release sender and arm a finite watchdog.
    pub(super) fn new(release: Sender<()>) -> Self {
        let (done_tx, done_rx) = bounded(1);
        let watchdog_release = release.clone();
        let watchdog = std::thread::spawn(move || {
            if done_rx.recv_timeout(flush_bound() * 3).is_err() {
                let _ = watchdog_release.try_send(());
            }
        });
        Self {
            done: done_tx,
            release,
            watchdog: Some(watchdog),
        }
    }
}

impl Drop for StallGuard {
    fn drop(&mut self) {
        let _ = self.done.try_send(());
        let _ = self.release.try_send(());
        if let Some(watchdog) = self.watchdog.take() {
            watchdog
                .join()
                .expect("stall watchdog should exit when the test guard drops");
        }
    }
}

/// Build a logger on a one-slot shard whose worker is held by a handler.
fn stalled_logger() -> (FemtoLogger, &'static Shard, StallGuard) {
    let shard = Box::leak(Box::new(
        Shard::start(0, 1).expect("a test shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("flush".to_string(), shard);
    let (started_tx, started_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(StallingHandler {
        started: started_tx,
        release: release_rx,
        released: AtomicBool::new(false),
    });
    logger
        .add_handler(handler)
        .expect("the native stalling handler should register");
    logger.log(crate::level::FemtoLevel::Info, "stall");
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the worker should enter the stalling handler");
    (logger, shard, StallGuard::new(release_tx))
}

/// Wait until the first flush has inserted its marker into the shard queue.
fn wait_for_queued_marker(shard: &Shard) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while shard.queued_jobs_for_test() == 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(
        shard.queued_jobs_for_test(),
        1,
        "the first flush marker should occupy the shallow shard queue"
    );
}

#[test]
fn flush_handlers_gives_up_when_a_full_queue_rejects_its_marker() {
    let (logger, shard, guard) = stalled_logger();

    // The worker is blocked on the record. The first flush fills the one-slot
    // queue with its marker; the second must time out while inserting another.
    std::thread::scope(|scope| {
        let first_flush = scope.spawn(|| logger.flush_handlers());
        wait_for_queued_marker(shard);

        let started_at = Instant::now();
        let flushed = logger.flush_handlers();
        let elapsed = started_at.elapsed();

        assert!(
            !flushed,
            "a flush whose marker cannot be enqueued must not report success"
        );
        assert!(
            elapsed < flush_bound(),
            "flush waited {elapsed:?} on a full queue; the bound is {:?}",
            flush_bound()
        );
        assert_eq!(
            shard.queued_jobs_for_test(),
            1,
            "a timed-out marker must stay within the shard's one-slot bound"
        );

        let repeated_started = Instant::now();
        assert!(
            !logger.flush_handlers(),
            "a repeated flush must fail while the stale marker occupies the queue"
        );
        assert!(
            repeated_started.elapsed() < flush_bound(),
            "a repeated full-queue flush must remain bounded"
        );
        assert_eq!(
            shard.queued_jobs_for_test(),
            1,
            "repeated timeouts must not add jobs beyond the bounded queue"
        );

        drop(guard);
        let _ = first_flush
            .join()
            .expect("the first flush call should leave after worker release");
    });
}

#[test]
fn flush_handlers_gives_up_on_a_stalled_handler_within_the_deadline() {
    let (logger, _shard, guard) = stalled_logger();

    let started_at = Instant::now();
    let flushed = logger.flush_handlers();
    let elapsed = started_at.elapsed();

    assert!(
        !flushed,
        "a flush the worker never acknowledges must not report success"
    );
    assert!(
        elapsed < flush_bound(),
        "flush waited {elapsed:?} on a stalled handler; the bound is {:?}",
        flush_bound()
    );
    drop(guard);
}

#[test]
fn flush_handlers_succeeds_once_the_worker_is_moving_again() {
    let (logger, _shard, guard) = stalled_logger();
    let _ = guard.release.send(());

    let started_at = Instant::now();
    let flushed = logger.flush_handlers();
    let elapsed = started_at.elapsed();

    assert!(flushed, "a flush with a moving worker should succeed");
    assert!(
        elapsed < flush_bound(),
        "a successful flush should not consume the whole budget, took {elapsed:?}"
    );
    drop(guard);
}

#[test]
fn flush_handlers_reports_a_handler_whose_flush_fails() {
    let shard = Box::leak(Box::new(
        Shard::start(0, 2).expect("a test shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("flush".to_string(), shard);
    let failing = Arc::new(FailingFlushHandler) as Arc<dyn FemtoHandlerTrait>;
    logger
        .add_handler(failing)
        .expect("the failing handler should register");

    assert!(
        !logger.flush_handlers(),
        "a handler whose flush fails must make the logger's flush fail"
    );
}

#[test]
fn flush_handlers_succeeds_with_no_handlers() {
    let shard = Box::leak(Box::new(
        Shard::start(0, 1).expect("a test shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("flush".to_string(), shard);

    assert!(logger.flush_handlers());
}
