//! Regression coverage for legacy custom handler flush compatibility.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::handler::{FemtoHandlerTrait, HandlerError};
use crate::logger::{FemtoLogger, Shard};
use crate::log_record::FemtoLogRecord;
use crate::sync::{Receiver, Sender, bounded};

use super::logger_tests_flush::{StallGuard, flush_bound};

/// Build a logger on an isolated shard for the compatibility contract tests.
fn isolated_logger() -> (FemtoLogger, &'static Shard) {
    let shard = Box::leak(Box::new(
        Shard::start(0, 4).expect("a test shard should start"),
    ));
    (
        FemtoLogger::assembled_for_test("legacy-flush".to_string(), shard),
        shard,
    )
}

/// Legacy handler whose only flush implementation blocks until released.
struct LegacyBlockingFlushHandler {
    /// Signals if the legacy method is invoked.
    started: Sender<()>,
    /// Keeps `flush()` blocked until the test explicitly releases it.
    release: Receiver<()>,
    /// Counts invocations so repeated failures can prove no work was started.
    calls: Arc<AtomicUsize>,
}

impl FemtoHandlerTrait for LegacyBlockingFlushHandler {
    fn handle(&self, _record: FemtoLogRecord) -> Result<(), HandlerError> {
        Ok(())
    }

    fn flush(&self) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.started.try_send(());
        let _ = self.release.recv();
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[test]
fn flush_handlers_fails_closed_for_a_blocking_legacy_flush() {
    let (started_tx, started_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(LegacyBlockingFlushHandler {
        started: started_tx,
        release: release_rx,
        calls: Arc::clone(&calls),
    });
    let (logger, _shard) = isolated_logger();
    logger
        .add_handler(handler)
        .expect("legacy test handler should be accepted");
    let _release_guard = StallGuard::new(release_tx);

    let started_at = Instant::now();
    let flushed = logger.flush_handlers();
    let elapsed = started_at.elapsed();

    assert!(
        !flushed,
        "a legacy handler without deadline support fails closed"
    );
    assert!(
        elapsed < flush_bound(),
        "flush waited {elapsed:?} for the legacy handler's explicit release"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the legacy synchronous flush must not be invoked"
    );
    assert!(
        started_rx.try_recv().is_err(),
        "the legacy handler must remain blocked until the test releases it"
    );
}

#[test]
fn repeated_legacy_flush_failures_start_no_background_work() {
    let (started_tx, started_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler: Arc<dyn FemtoHandlerTrait> = Arc::new(LegacyBlockingFlushHandler {
        started: started_tx,
        release: release_rx,
        calls: Arc::clone(&calls),
    });
    let (logger, shard) = isolated_logger();
    logger
        .add_handler(handler)
        .expect("legacy test handler should be accepted");
    let _release_guard = StallGuard::new(release_tx);

    let started_at = Instant::now();
    for _ in 0..16 {
        assert!(
            !logger.flush_handlers(),
            "a legacy handler without deadline support must fail closed"
        );
    }
    let elapsed = started_at.elapsed();

    assert!(
        elapsed < flush_bound(),
        "repeated fail-closed calls took {elapsed:?}"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "repeated calls must not spawn or invoke legacy flush work"
    );
    assert!(started_rx.try_recv().is_err());
    assert!(
        shard.queued_jobs_for_test() <= 4,
        "repeated calls must remain within the shard's bounded queue"
    );
}
