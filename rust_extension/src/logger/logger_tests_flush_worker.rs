//! Regression coverage for worker-owned flush snapshots and GIL boundaries.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::handler::{FemtoHandlerTrait, HandlerError, remaining};
use crate::log_record::FemtoLogRecord;
use crate::sync::{Receiver, Sender, bounded};

use super::logger_tests_flush::flush_bound;
use super::{FemtoLogger, Shard};

/// A deliberately noncompliant callback that occupies the one logger worker.
struct BlockingFlushUntilHandler {
    /// Signals when the worker enters the callback.
    started: Sender<()>,
    /// Keeps the callback blocked until the test releases the worker.
    release: Receiver<()>,
    /// Counts callback executions while concurrent barriers time out.
    calls: Arc<AtomicUsize>,
}

impl FemtoHandlerTrait for BlockingFlushUntilHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, _deadline: Instant) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.started.try_send(());
        let _ = self.release.recv();
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Releases a blocked worker during normal return or assertion unwinding.
struct FlushCallbackReleaseGuard(Sender<()>);

impl Drop for FlushCallbackReleaseGuard {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

/// Handler whose destructor blocks so the test can identify which thread drops it.
struct BlockingDropFlushHandler {
    /// Signals that the marker reached the handler flush callback.
    flush_started: Sender<()>,
    /// Holds the flush callback open until the test removes the handler.
    flush_release: Receiver<()>,
    /// Signals that the worker began dropping its marker-owned snapshot.
    drop_started: Sender<()>,
    /// Keeps destruction blocked until the test explicitly releases it.
    drop_release: Receiver<()>,
}

impl FemtoHandlerTrait for BlockingDropFlushHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush_until(&self, deadline: Instant) -> bool {
        let _ = self.flush_started.try_send(());
        let _ = self
            .flush_release
            .recv_timeout(remaining(deadline, Instant::now()));
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for BlockingDropFlushHandler {
    fn drop(&mut self) {
        let _ = self.drop_started.try_send(());
        let _ = self.drop_release.recv();
    }
}

/// Releases a test destructor even while unwinding after a failed assertion.
struct WorkerReleaseGuard {
    /// Releases the callback if an assertion unwinds before the test does.
    flush: Sender<()>,
    /// Releases the worker-side destructor if an assertion unwinds.
    drop_handler: Sender<()>,
}

impl WorkerReleaseGuard {
    /// Let the callback finish after the test has removed its handler.
    fn release_flush(&self) {
        let _ = self.flush.try_send(());
    }
}

impl Drop for WorkerReleaseGuard {
    fn drop(&mut self) {
        let _ = self.flush.try_send(());
        let _ = self.drop_handler.try_send(());
    }
}

#[test]
fn flush_handler_destruction_does_not_extend_the_caller_wait() {
    let (flush_started_tx, flush_started_rx) = bounded(1);
    let (flush_release_tx, flush_release_rx) = bounded(1);
    let (drop_started_tx, drop_started_rx) = bounded(1);
    let (drop_release_tx, drop_release_rx) = bounded(1);
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(BlockingDropFlushHandler {
        flush_started: flush_started_tx,
        flush_release: flush_release_rx,
        drop_started: drop_started_tx,
        drop_release: drop_release_rx,
    });
    let logger = FemtoLogger::new("flush-drop".to_string())
        .expect("flush-drop test logger should be constructible");
    logger
        .add_handler(handler)
        .expect("test handler should be accepted");

    std::thread::scope(|scope| {
        let release_guard = WorkerReleaseGuard {
            flush: flush_release_tx,
            drop_handler: drop_release_tx,
        };
        let (result_tx, result_rx) = bounded(1);
        let logger_ref = &logger;
        let started_at = Instant::now();
        let caller = scope.spawn(move || {
            let _ = result_tx.send(logger_ref.flush_handlers());
        });

        flush_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker should start the flush callback");
        let (removed_tx, removed_rx) = bounded(1);
        let logger_ref = &logger;
        let remover = scope.spawn(move || {
            logger_ref.clear_handlers();
            let _ = removed_tx.send(());
        });
        removed_rx
            .recv_timeout(flush_bound())
            .expect("removing a handler must not wait for its callback");
        remover
            .join()
            .expect("handler removal thread should finish");
        release_guard.release_flush();

        let flushed = result_rx
            .recv_timeout(flush_bound())
            .expect("the caller should not wait for the handler destructor");
        let elapsed = started_at.elapsed();
        assert!(!flushed, "the handler reports an unconfirmed flush");
        assert!(elapsed < flush_bound());
        drop_started_rx
            .recv_timeout(flush_bound())
            .expect("the marker-owned handler should be dropped on the worker");

        caller
            .join()
            .expect("flush caller should return before its handler is released");
    });
}

#[test]
fn a_full_marker_queue_does_not_hold_the_handler_registry_lock() {
    let shard = Box::leak(Box::new(
        Shard::start(0, 2).expect("isolated flush shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("flush-registry-lock".to_string(), shard);
    let (started_tx, started_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    logger
        .add_handler(Arc::new(BlockingFlushUntilHandler {
            started: started_tx,
            release: release_rx,
            calls: Arc::new(AtomicUsize::new(0)),
        }))
        .expect("test handler should attach");

    std::thread::scope(|scope| {
        let release_guard = FlushCallbackReleaseGuard(release_tx);
        let first_logger = &logger;
        let first = scope.spawn(move || first_logger.flush_handlers());
        started_rx
            .recv_timeout(flush_bound())
            .expect("the first marker should start the blocked flush callback");

        let second_logger = &logger;
        let second = scope.spawn(move || second_logger.flush_handlers());
        let queue_deadline = Instant::now() + flush_bound();
        while shard.queued_jobs_for_test() != 1 && Instant::now() < queue_deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            shard.queued_jobs_for_test(),
            1,
            "the second marker should occupy the remaining queue slot"
        );

        let (third_started_tx, third_started_rx) = bounded(1);
        let third_logger = &logger;
        let third = scope.spawn(move || {
            let _ = third_started_tx.send(());
            third_logger.flush_handlers()
        });
        third_started_rx
            .recv_timeout(flush_bound())
            .expect("the third flush caller should start");
        // Give the caller time to reach the full queue's timed send.
        std::thread::sleep(Duration::from_millis(100));

        let (removed_tx, removed_rx) = bounded(1);
        let remover_logger = &logger;
        let remover = scope.spawn(move || {
            remover_logger.clear_handlers();
            let _ = removed_tx.send(());
        });
        removed_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("queue backpressure must not retain the handler-list read lock");
        remover
            .join()
            .expect("handler removal should finish while marker insertion waits");

        drop(release_guard);
        assert!(!first.join().expect("first flush caller should finish"));
        assert!(!second.join().expect("second flush caller should finish"));
        assert!(!third.join().expect("third flush caller should finish"));
    });
}

#[test]
fn concurrent_stalled_flushes_keep_work_on_the_single_bounded_worker() {
    const FLUSH_CALLS: usize = 12;

    let (started_tx, started_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    let calls = Arc::new(AtomicUsize::new(0));
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(BlockingFlushUntilHandler {
        started: started_tx,
        release: release_rx,
        calls: Arc::clone(&calls),
    });
    let shard = Box::leak(Box::new(
        Shard::start(0, FLUSH_CALLS + 1).expect("isolated flush shard should start"),
    ));
    let logger = FemtoLogger::assembled_for_test("flush-concurrent-timeouts".to_string(), shard);
    logger
        .add_handler(handler)
        .expect("test handler should be accepted");

    std::thread::scope(|scope| {
        // Keep the cleanup guard inside the scope closure: if an assertion
        // fails, it releases the worker before scoped threads are joined.
        let _release_guard = FlushCallbackReleaseGuard(release_tx);
        let start = Arc::new(std::sync::Barrier::new(FLUSH_CALLS));
        let (results_tx, results_rx) = bounded(FLUSH_CALLS);

        for _ in 0..FLUSH_CALLS {
            let start = Arc::clone(&start);
            let logger = &logger;
            let results_tx = results_tx.clone();
            scope.spawn(move || {
                start.wait();
                let started_at = Instant::now();
                let flushed = logger.flush_handlers();
                let _ = results_tx.send((flushed, started_at.elapsed()));
            });
        }

        started_rx
            .recv_timeout(flush_bound())
            .expect("the single logger worker should enter the blocked callback");

        for _ in 0..FLUSH_CALLS {
            let (flushed, elapsed) = results_rx
                .recv_timeout(flush_bound())
                .expect("every concurrent barrier should give up by its deadline");
            assert!(!flushed, "an unconfirmed callback must fail closed");
            assert!(
                elapsed < flush_bound(),
                "a caller waited {elapsed:?} for a callback that has not been released"
            );
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "timed-out barriers must not create concurrent flush callbacks"
        );
    });
}

#[cfg(feature = "python")]
#[test]
fn native_flush_does_not_attach_to_the_gil() {
    use pyo3::Python;

    /// Signals when a native flush callback reaches the worker under a held GIL.
    struct FlushSignalHandler(Sender<()>);

    impl FemtoHandlerTrait for FlushSignalHandler {
        fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
            Ok(())
        }

        fn flush_until(&self, _deadline: Instant) -> bool {
            let _ = self.0.try_send(());
            true
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    let logger = Arc::new(
        FemtoLogger::new("flush-gil".to_string())
            .expect("GIL flush test logger should be constructible"),
    );
    let (flush_started_tx, flush_started_rx) = bounded(1);
    logger
        .add_handler(Arc::new(FlushSignalHandler(flush_started_tx)))
        .expect("native flush handler should attach");
    let (result_tx, result_rx) = bounded(1);

    let (result, caller) = Python::attach(|_py| {
        let logger = Arc::clone(&logger);
        let caller = std::thread::spawn(move || {
            let _ = result_tx.send(logger.flush_handlers());
        });
        let callback = flush_started_rx.recv_timeout(flush_bound());
        (
            callback.and_then(|()| result_rx.recv_timeout(flush_bound())),
            caller,
        )
    });

    caller
        .join()
        .expect("native flush thread should finish after interpreter release");
    assert!(
        result.is_ok_and(|flushed| flushed),
        "native Rust flush must not wait to attach while another thread owns the GIL"
    );
}
