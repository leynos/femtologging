//! Regression coverage for worker-owned flush snapshots and GIL boundaries.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::handler::{FemtoHandlerTrait, HandlerError, remaining};
use crate::log_record::FemtoLogRecord;
use crate::sync::{Receiver, Sender, bounded};

use super::FemtoLogger;
use super::logger_tests_flush::flush_bound;

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
        let _ = self.flush_release.recv_timeout(remaining(deadline));
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
    let logger = FemtoLogger::new("flush-drop".to_string());
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
    let logger = FemtoLogger::new("flush-concurrent-timeouts".to_string());
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

    let logger = Arc::new(FemtoLogger::new("flush-gil".to_string()));
    let (result_tx, result_rx) = bounded(1);

    let (early_result, caller) = Python::attach(|_py| {
        let logger = Arc::clone(&logger);
        let caller = std::thread::spawn(move || {
            let _ = result_tx.send(logger.flush_handlers());
        });
        (result_rx.recv_timeout(Duration::from_millis(250)), caller)
    });

    let late_result = early_result
        .is_err()
        .then(|| result_rx.recv_timeout(Duration::from_secs(5)));
    caller
        .join()
        .expect("native flush thread should finish after interpreter release");
    assert!(
        early_result.is_ok_and(|flushed| flushed),
        "native Rust flush must not wait to attach while another thread owns the GIL"
    );
    if let Some(result) = late_result {
        assert!(result.is_ok_and(|flushed| flushed));
    }
}
